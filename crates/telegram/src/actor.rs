use std::io::{self, Write};

use crate::{
    TelegramHandle,
    call::{ActiveCall, UpdateOutcome},
    types::{AudioFrame, TelegramCommand, TelegramEvent},
};
use anyhow::Result;
use config::BridgeConfig;
use ferogram::{Client, SignInError};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub struct TelegramActor {
    client: Client,
    cmd_rx: mpsc::Receiver<TelegramCommand>,
    event_tx: mpsc::Sender<TelegramEvent>,
    audio_in_tx: mpsc::Sender<AudioFrame>,
    audio_out_rx: mpsc::Receiver<AudioFrame>,
}

impl TelegramActor {
    pub fn new(
        client: Client,
        cmd_rx: mpsc::Receiver<TelegramCommand>,
        event_tx: mpsc::Sender<TelegramEvent>,
        audio_in_tx: mpsc::Sender<AudioFrame>,
        audio_out_rx: mpsc::Receiver<AudioFrame>,
    ) -> Self {
        Self {
            client,
            cmd_rx,
            event_tx,
            audio_in_tx,
            audio_out_rx,
        }
    }

    /// Consume the actor and run it on the current task.
    /// Call this inside `tokio::spawn`.
    pub async fn run(self) -> anyhow::Result<()> {
        let Self {
            client,
            mut cmd_rx,
            event_tx,
            audio_in_tx,
            audio_out_rx,
        } = self;
        let mut stream = client.stream_updates();

        let mut audio_in_tx = Some(audio_in_tx);
        let mut audio_out_rx = Some(audio_out_rx);

        // ── IDLE: no active call, we own the stream ───────────────────────
        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        TelegramCommand::Dial { user_id } => {
                            let _ = event_tx.send(TelegramEvent::Ringing { user_id }).await;

                            // Hand stream off to the call — we go dark on updates
                            // until run_signaling returns.
                            let result = run_call(
                                &client,
                                user_id,
                                &mut stream,
                                &mut cmd_rx,
                                &event_tx,
                                audio_in_tx.take().expect("audio_in_tx already consumed"),
                                audio_out_rx.take().expect("audio_out_rx already consumed"),
                            ).await;

                            match result {
                                Ok(()) => {}
                                Err(e) => {
                                    let _ = event_tx.send(
                                        TelegramEvent::Error(e.to_string())
                                    ).await;
                                }
                            }
                            // TODO: on harness, when attempting to redial after a hangup
                            // getting the None Variant for `audio_in_tx.take()`
                            // stream is ours again — back to idle
                        }
                        TelegramCommand::Shutdown => break,
                        // Hangup with no active call: ignore
                        TelegramCommand::Hangup => {}
                    }
                }

                // TODO: here we should listen to update stream and see if we get a call
                // No commands and stream closed → session died
                else => break,
            }
        }
        Ok(())
    }
}

pub fn spawn(client: Client) -> (TelegramHandle, TelegramActor) {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (event_tx, event_rx) = mpsc::channel(32);
    let (audio_in_tx, audio_in_rx) = mpsc::channel(256);
    let (audio_out_tx, audio_out_rx) = mpsc::channel(256);

    let actor = TelegramActor::new(client, cmd_rx, event_tx, audio_in_tx, audio_out_rx);
    let handle = TelegramHandle {
        cmd_tx,
        event_rx,
        audio_in_rx,
        audio_out_tx,
    };

    (handle, actor)
}

/// Drives a single outbound call from dial → connected → ended.
/// Owns `stream` for its entire duration.
async fn run_call(
    client: &Client,
    user_id: i64,
    stream: &mut ferogram::UpdateStream,
    cmd_rx: &mut mpsc::Receiver<TelegramCommand>,
    event_tx: &mpsc::Sender<TelegramEvent>,
    audio_in_tx: mpsc::Sender<AudioFrame>,
    audio_out_rx: mpsc::Receiver<AudioFrame>,
) -> Result<()> {
    // ── DIAL: request call parameters, stream is consumed here ───────────
    let (mut active, servers, versions) = ActiveCall::dial(client, user_id, stream).await?;

    // ── CONNECT: establish WebRTC transport ───────────────────────────────
    let (mut sig_out_rx, mut conn_rx) = active.connect(&servers, &versions).await?;

    info!("preparing media");
    active.prepare_media(audio_in_tx).await?;

    // ── SIGNALING: stream is consumed here until ICE+DTLS resolves ────────
    // let connected = tokio::select! {
    //     result = active.run_signaling(&mut sig_out_rx, &mut conn_rx, stream) => {
    //         result?
    //     }
    //     Some(TelegramCommand::Hangup) | Some(TelegramCommand::Shutdown) = cmd_rx.recv() => {
    //         active.end().await;
    //         let _ = event_tx.send(TelegramEvent::CallEnded {
    //             user_id,
    //             reason: Some("local hangup".into()),
    //         }).await;
    //         return Ok(());
    //     }
    // };
    let connected = active
        .run_signaling(&mut sig_out_rx, &mut conn_rx, stream)
        .await?;

    info!("signaling done, connected: {connected}");
    if !connected {
        active.end().await;
        let _ = event_tx
            .send(TelegramEvent::CallEnded {
                user_id,
                reason: Some("ICE/DTLS failed".into()),
            })
            .await;
        return Ok(());
    }

    // ── CONNECTED: wire up audio, stream is ours again ────────────────────
    let _ = event_tx
        .send(TelegramEvent::CallConnected { user_id })
        .await;

    info!("activating audio");
    active.activate_audio(audio_out_rx).await?;

    // ── ACTIVE: watch for hangup or remote disconnect ─────────────────────
    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    TelegramCommand::Hangup | TelegramCommand::Shutdown => {
                        active.end().await;
                        let _ = event_tx.send(TelegramEvent::CallEnded {
                            user_id,
                            reason: Some("local hangup".into()),
                        }).await;
                        return Ok(());
                    }
                    _ => {}
                }
            }

            // Telegram → ntgcalls (and remote hangup)
            maybe_upd = stream.next_raw() => match maybe_upd {
                None => { active.end().await; return Ok(()); }
                Some(upd) => {
                    if let UpdateOutcome::Discarded = active.handle_update(upd).await? {
                        active.end().await;
                        let _ = event_tx.send(TelegramEvent::CallEnded {
                            user_id, reason: Some("remote hangup".into()),
                        }).await;
                        return Ok(());
                    }
                }
            },

            // connection state changes after the first "connected"
            Some(connected) = conn_rx.recv() => {
                if !connected {
                    active.end().await;
                    let _ = event_tx.send(TelegramEvent::CallEnded {
                        user_id, reason: Some("connection lost".into()),
                    }).await;
                    return Ok(());
                }
            },

            Some(bytes) = sig_out_rx.recv() => {
                active.send_signaling(&bytes).await?;
            }
        }
    }
}

// TODO: add doc comments
pub async fn login(config: &BridgeConfig) -> Result<(), anyhow::Error> {
    let (client, _shutdown) = ferogram::Client::builder()
        .api_id(config.telegram.api_id)
        .api_hash(&config.telegram.api_hash)
        .session(&config.telegram.session_file)
        .connect()
        .await?;

    let token = match client
        .request_login_code(&config.telegram.phone_num)
        .await?
    {
        ferogram::SendCodeOutcome::CodeRequired(token) => token,
        ferogram::SendCodeOutcome::AlreadyAuthorized(user) => {
            info!("recovered login from logout token, welcome back {user}");
            return Ok(());
        }
    };

    let code = capture_from_stdin("Telegram login code: ");

    match client.sign_in(&token, &code).await {
        Ok(name) => info!("logged in as {name}"),
        Err(SignInError::PasswordRequired(pw_token)) => {
            let password = capture_from_stdin("Telegram password: "); // TODO: hide password chars
            client.check_password(*pw_token, password.trim()).await?;
        }
        Err(err) => {
            return Err(err.into());
        }
    }
    Ok(())
}

fn capture_from_stdin(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().expect("Failed to flush stdout");

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .expect("Failed to read line");
    input.trim().to_string()
}
