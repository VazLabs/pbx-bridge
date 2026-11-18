use std::sync::Arc;

use anyhow::Result;
use asterisk::{AsteriskCommand, ChannelId};
use clap::{Parser, Subcommand};
use config::BridgeConfig;
use telegram::{TelegramCommand, TelegramEvent, login};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "bridge")]
#[command(about = "Bridges SIP into different protocols")]
#[command(version)]
#[command(subcommand_required = false)]
struct BinArgs {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    RunBridge,
    Login,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    config::config_tracing().await?;

    let args = BinArgs::parse();

    let bridge_config = match config::load("bridge_config.toml") {
        Ok(config) => config,
        Err(e) => {
            error!(error = %e, "failed to load bridge config on startup");
            std::process::exit(1);
        }
    };

    match args.command.unwrap_or(Commands::RunBridge) {
        Commands::RunBridge => {
            info!("running bridge");
            match bridge(&bridge_config).await {
                Ok(_) => info!("shutting down, goodbye"),
                Err(err) => {
                    error!("error: {err}");
                    error!("main bridge returned an error");
                }
            };
        }

        Commands::Login => {
            info!("attempting login as {}", bridge_config.telegram.phone_num);
            match login(&bridge_config.telegram).await {
                Ok(_) => info!("logged in successful"),
                Err(err) => {
                    error!("error: {err}");
                    error!("Login unsucessful, try again")
                }
            };
        }
    }
    Ok(())
}

async fn bridge(config: &BridgeConfig) -> Result<()> {
    let (tg_client, mut tg_handler) = telegram::TelegramClient::init(&config.telegram).await?;

    let (mut asterisk_client, mut asterisk_handler) =
        asterisk::AsteriskClient::init(config.asterisk.clone());

    let tg_thread_handle = tg_client.spawn_local_client();
    let asterisk_task_handle = tokio::spawn(async move {
        let result = asterisk_client.run().await;
        if let Err(e) = &result {
            error!("asterisk client.run() failed: {e}");
        }
        result
    });

    let client_channel_id: Arc<Mutex<Option<ChannelId>>> = Arc::new(Mutex::new(None)); // change to regular mutex

    loop {
        tokio::select! {
            biased;


            _ = tokio::signal::ctrl_c() => {
                warn!("SIGINT received, shutting down");
                match tg_handler.cmd_tx.send(TelegramCommand::Shutdown).await {
                    Ok(_) => {},
                    Err(e) => {
                        error!("telegram cmd_tx channel closed");
                        anyhow::bail!(e);
                    },
                };
                match asterisk_handler.cmd_tx.send(AsteriskCommand::Shutdown).await {
                    Ok(_) => {},
                    Err(e) => {
                        error!("asterisk cmd_tx channel closed");
                        anyhow::bail!(e);
                    },
                };
                break;
            }

            Some(event) = asterisk_handler.event_rx.recv() => {
                match event {
                    asterisk::AsteriskEvent::CallRequested { channel_id, call_num, audio_rx, bridge: _ } => {
                        info!("outgoing call to user {call_num} ");

                        let Some(tg_contact) = router::evaluate_outbound(call_num as u64, &config.contacts) else {
                            warn!("number {call_num} is not a contact");
                            // TODO: give out a wrong number dial tone or something
                            continue;
                        };

                        {
                            let mut guard = client_channel_id.lock().await;
                            *guard = Some(channel_id);
                        }

                        let channels = telegram::prepare_audio_channels();

                        tokio::spawn(async move {
                            if let Err(e) = bridge::asterisk_audio_plane(audio_rx, channels.inbound_channels, channels.cancellation_token.clone()).await {
                                error!("asterisk outgoing task error: {e}");
                            }
                        });

                        tg_handler.cmd_tx.send(telegram::TelegramCommand::Dial { user_id: tg_contact.into(), channels: channels.outbound_channels }).await?;

                    },
                    asterisk::AsteriskEvent::CallEnded => {
                        info!("phone hung up, call has ended");
                        tg_handler.cmd_tx.send(telegram::TelegramCommand::Hangup).await?;
                    },
                }
            }

            Some(event) = tg_handler.event_rx.recv() => {
                match event {
                    TelegramEvent::CallConnected { user_id: _ } => {
                        info!("Telegram Call connected");

                        let guard = client_channel_id.lock().await;
                        match &*guard  {
                            Some(channel_id) => {
                                asterisk_handler.cmd_tx.send(AsteriskCommand::AcceptCall(channel_id.clone())).await?;
                            },
                            None => error!("Call connected with no asterist channel ID in store"),
                        }
                    },
                    TelegramEvent::CallEnded { user_id: _, reason } => {
                        info!("[telegram] call ended: {reason:?}");
                        let guard = client_channel_id.lock().await;
                        match &*guard  {
                            Some(channel_id) => {
                                asterisk_handler.cmd_tx.send(AsteriskCommand::Hangup(channel_id.clone())).await?;
                            },
                            None => error!("Call connected with no asterist channel ID in store"),
                        }
                    },
                    _ => {} // TODO: finish events
                    // TelegramEvent::IncomingCall { user_id, call_id } => todo!(),
                    // TelegramEvent::Ringing { user_id } => todo!(),
                    // TelegramEvent::FatalError(error) => todo!(),
                    // TelegramEvent::Error(_) => todo!(),
                }
            }
        }
    }

    let _ = asterisk_task_handle.await?;
    tg_thread_handle.join().expect("telegram thread panicked");

    Ok(())
}
