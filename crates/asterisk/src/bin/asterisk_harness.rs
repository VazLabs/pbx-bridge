use std::{collections::VecDeque, sync::Arc};

use anyhow::Result;
use asterisk::{AsteriskClient, AsteriskCommand, AudioSocketMessage, ChannelId};
use config::BridgeConfig;
use tokio::{io::AsyncBufReadExt, sync::Mutex};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    config::config_tracing().await?;

    let bridge_config = match config::load("bridge_config.toml") {
        Ok(config) => config,
        Err(e) => {
            error!(error = %e, "failed to load bridge config on startup");
            std::process::exit(1);
        }
    };

    harness_main(&bridge_config).await?;
    // example(&bridge_config).await?;
    Ok(())
}

async fn harness_main(config: &BridgeConfig) -> anyhow::Result<()> {
    let config = config.asterisk.clone();
    let (mut client, mut handler) = AsteriskClient::init(config);

    let client_task = tokio::spawn(async move {
        let result = client.run().await;
        if let Err(e) = &result {
            error!("client.run() failed: {e}");
        }
        result
    });

    let harness_channel_id: Arc<Mutex<Option<ChannelId>>> = Arc::new(Mutex::new(None));

    let cmd_tx = handler.cmd_tx.clone();
    info!("Commands: 0=shutdown, 1=accept call, 2=hangup call");

    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin).lines();

    loop {
        tokio::select! {
            Ok(Some(line)) = reader.next_line() => {
                match line.trim().parse().unwrap_or(99u8) {
                    0 => {
                        cmd_tx.send(AsteriskCommand::Shutdown).await?;
                        break;
                    }
                    1 => {
                        // the bridge will need to set everything up, and give the OK to answer the call
                        // all audio sockets need to be ready _before_ we send the Answer API call
                        let guard = harness_channel_id.lock().await;
                        match &*guard {
                            Some(id) => cmd_tx.send(AsteriskCommand::AcceptCall(id.clone())).await?,
                            None => error!("called Asnwer call with not channel ID in store"),
                        };

                    }
                    2 => {
                        let guard = harness_channel_id.lock().await;
                        match &*guard {
                            Some(id) => cmd_tx.send(AsteriskCommand::Hangup(id.clone())).await?,
                            None => warn!("called Hangup with no channel id in store"),
                        }
                    }
                    _ => info!("unknown command"),

                }
            }

            Some(event) = handler.event_rx.recv() => {
                info!("[event] {event:?}");
                match event {
                    asterisk::AsteriskEvent::CallRequested {channel_id , call_num, audio_rx, bridge: _ } => {
                        info!("call requested to user {call_num}, on channel_id: {channel_id}");
                        let channel_id_ref = Arc::clone(&harness_channel_id);
                        tokio::spawn(async move {
                            let Ok(mut audio_socket) = audio_rx.await else {
                                warn!("AudioSocket never connected");
                                return;
                            };
                            info!("AudioSocket stream ready, starting media loop");

                            {
                                let mut guard = channel_id_ref.lock().await;
                                *guard = Some(channel_id);
                            }

                            const ECHO_DELAY_FRAMES: usize = 50;
                            let mut echo: VecDeque<Vec<u8>> = VecDeque::new();

                            while let Some(msg) = audio_socket.next_message().await {
                                match msg {
                                    AudioSocketMessage::Dtmf(c) => info!(%c, "DTMF received"),
                                    AudioSocketMessage::Terminate | AudioSocketMessage::Error(_) => break,
                                    AudioSocketMessage::Audio(_, frame) => {
                                        echo.push_back(frame);
                                        if echo.len() > ECHO_DELAY_FRAMES {
                                            audio_socket.push_audio(echo.pop_front().expect("failed to pop front in echo"));
                                        }
                                    },
                                    AudioSocketMessage::StreamId(uuid) => info!(%uuid, "AudioSocket handshake"),
                                }
                            }

                            let mut guard = channel_id_ref.lock().await;
                            *guard = None;
                        });

                    },
                    asterisk::AsteriskEvent::CallEnded => {
                        info!("phone hung up, call has ended");
                    },
                }

            }
            else => {
                error!("all channels closed — client likely shut down unexpectedly");
                break;
            }
        }
    }

    let _ = client_task.await?;
    Ok(())
}
