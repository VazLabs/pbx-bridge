use std::io::Write;

use anyhow::{Result, bail};
use config::TelegramConfig;
use tokio::{sync::mpsc, task::LocalSet};
use tracing::{debug, error, info, warn};

use crate::{
    AudioFrame, TelegramCommand, TelegramEvent,
    call::{ActiveCall, UpdateOutcome},
    types::UserId,
};

pub struct TelegramClient {
    client: ferogram::Client,
    cmd_rx: mpsc::Receiver<TelegramCommand>,
    event_tx: mpsc::Sender<TelegramEvent>,
}

pub struct TelegramHandler {
    pub cmd_tx: mpsc::Sender<TelegramCommand>,
    pub event_rx: mpsc::Receiver<TelegramEvent>,
}

impl TelegramClient {
    pub async fn init(config: &TelegramConfig) -> Result<(TelegramClient, TelegramHandler)> {
        let (client, _shutdown) = ferogram::Client::builder()
            .api_id(config.api_id)
            .api_hash(&config.api_hash)
            .session(&config.session_file)
            .connect()
            .await?;

        let (cmd_tx, cmd_rx) = mpsc::channel(200);
        let (event_tx, event_rx) = mpsc::channel(200);

        let client = TelegramClient {
            client,
            cmd_rx,
            event_tx,
        };

        let handler = TelegramHandler { cmd_tx, event_rx };

        Ok((client, handler))
    }

    pub async fn run(self) -> Result<()> {
        let TelegramClient {
            client,
            mut cmd_rx,
            event_tx,
        } = self;

        let mut stream = client.stream_updates();

        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        TelegramCommand::Dial { user_id, channels } => {
                            let _ = event_tx.send(TelegramEvent::Ringing { user_id }).await;

                            let result = run_call(
                                &client,
                                user_id,
                                &mut stream,
                                &mut cmd_rx,
                                &event_tx,
                                channels.tx,
                                channels.rx
                            ).await;

                            match result {
                                Ok(_) => {},
                                Err(e) => {
                                    let _ = event_tx.send(TelegramEvent::Error(e.to_string())).await;
                                },
                            }
                        },
                        TelegramCommand::Shutdown => {
                            info!("shutdown command received");
                            break
                        },
                        // Hangup with no active call: ignore
                        TelegramCommand::Hangup => {}
                    }
                }

                // TODO: here we should listen to update stream and see if we get a call
                // No commands and stream closed -> session dies
                else => break,
            }
        }

        Ok(())
    }

    pub fn spawn_local_client(self) -> std::thread::JoinHandle<()> {
        std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build Telegram runtime"); // TODO: handle panic

            let local = LocalSet::new();
            local.spawn_local(async move {
                if let Err(e) = self.run().await {
                    error!("[telegram] client has died: {e}");
                }
            });

            rt.block_on(local)
        })
    }
}

async fn run_call(
    client: &ferogram::Client,
    user_id: UserId,
    stream: &mut ferogram::UpdateStream,
    cmd_rx: &mut mpsc::Receiver<TelegramCommand>,
    event_tx: &mpsc::Sender<TelegramEvent>,
    audio_in_tx: mpsc::Sender<AudioFrame>,
    audio_out_rx: mpsc::Receiver<AudioFrame>,
) -> Result<()> {
    let result = ActiveCall::dial(client, user_id, stream).await;

    if let Err(e) = &result {
        event_tx
            .send(TelegramEvent::CallEnded {
                user_id,
                reason: Some(format!("closed when dialing: {e}")),
            })
            .await?;
    }

    let (mut active, servers, versions) = result?;
    let (mut sig_out_rx, mut conn_rx) = active.connect(&servers, &versions).await?;

    debug!("preparing media");
    active.prepare_media(audio_in_tx).await?;

    let connected = active
        .run_signaling(&mut sig_out_rx, &mut conn_rx, stream)
        .await?;

    debug!("signaling done, connected: {connected}");
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

    let _ = event_tx
        .send(TelegramEvent::CallConnected { user_id })
        .await;

    debug!("activating audio");
    let token = tokio_util::sync::CancellationToken::new();
    active.activate_audio(audio_out_rx, token.clone()).await?;

    loop {
        tokio::select! {
            biased;

            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    TelegramCommand::Hangup | TelegramCommand::Shutdown => {
                        token.cancel();
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

            // connection state changes after the first "connected"
            Some(connected) = conn_rx.recv() => {
                if !connected {
                    token.cancel();
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

            // Telegram → ntgcalls (and remote hangup)
            maybe_upd = stream.next_raw() => match maybe_upd {
                None => {
                    active.end().await;
                    return Ok(());
                }
                Some(upd) => {
                    if let UpdateOutcome::Discarded = active.handle_update(upd).await? {
                        token.cancel();
                        active.end().await;
                        let _ = event_tx.send(TelegramEvent::CallEnded {
                            user_id, reason: Some("remote hangup".into()),
                        }).await;
                        return Ok(());
                    }
                }
            },
        }
    }
}

// TODO: add doc comments
pub async fn login(config: &TelegramConfig) -> Result<()> {
    let (client, _shutdown) = ferogram::Client::builder()
        .api_id(config.api_id)
        .api_hash(&config.api_hash)
        .session(&config.session_file)
        .connect()
        .await?;

    let token = match client.request_login_code(&config.phone_num).await? {
        ferogram::SendCodeOutcome::CodeRequired(token) => token,
        ferogram::SendCodeOutcome::AlreadyAuthorized(user) => {
            info!("recovered login from logout token, welcome back {user}");
            return Ok(());
        }
    };

    let code = capture_nonempty_stdin("Telegram login code: ");

    match client.sign_in(&token, &code).await {
        Ok(name) => info!("logged in as {name}"),
        Err(ferogram::SignInError::PasswordRequired(pw_token)) => {
            let password = capture_nonempty_stdin("Telegram password: "); // TODO: hide password chars
            client.check_password(*pw_token, password.trim()).await?;
        }
        Err(err) => {
            return Err(err.into());
        }
    }

    const MAX_RETRIES: u32 = 5;

    let mut attempts = 0;
    loop {
        match client.save_session().await {
            Ok(()) => break,
            Err(e) if attempts >= MAX_RETRIES => {
                // propagate or panic after exhausting retries
                error!("operation failed after {MAX_RETRIES} retries: {e}");
                bail!(e);
            }
            Err(e) => {
                attempts += 1;
                warn!("attempt {attempts} failed: {e}, retrying...");
            }
        }
    }

    client.disconnect();
    Ok(())
}

fn capture_from_stdin(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn capture_nonempty_stdin(prompt: &str) -> String {
    loop {
        match capture_from_stdin(prompt) {
            Ok(c) if !c.is_empty() => break c,
            Ok(_) => {}
            Err(e) => info!("error capturing from stdin: {e}"),
        }
    }
}
