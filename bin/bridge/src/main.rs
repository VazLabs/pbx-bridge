use std::sync::Arc;

use anyhow::Result;
use asterisk::{AsteriskCommand, AsteriskHandler, ChannelId};
use bridge::{ActorHandlers, handle_asterisk_outgoing, run_connections};
use clap::{Parser, Subcommand};
use config::BridgeConfig;
use telegram::{TelegramCommand, TelegramEvent, TelegramHandle, login};
use tokio::sync::{
    Mutex,
    mpsc::{self, Sender},
};
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
            match bridge_main(&bridge_config).await {
                Ok(_) => info!("shutting down, goodbye"),
                Err(err) => {
                    error!("error: {err}");
                    error!("main bridge returned an error");
                }
            };
        }

        Commands::Login => {
            info!("attempting login as {}", bridge_config.telegram.phone_num);
            match login(&bridge_config).await {
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

async fn bridge_main(config: &BridgeConfig) -> Result<()> {
    let (task_handlers, actor_handlers) = run_connections(config).await?;

    let asterisk_client_task = task_handlers.asterisk;
    let telegram_client_task = task_handlers.telegram;

    let ActorHandlers { asterisk, telegram } = actor_handlers;

    let AsteriskHandler {
        cmd_tx: a_cmd_tx,
        event_rx: mut a_event_rx,
    } = asterisk;

    let TelegramHandle {
        cmd_tx: tg_cmd_tx,
        event_rx: mut tg_event_rx,
        audio_in_rx: mut tg_audio_in_rx,
        audio_out_tx: tg_audio_out_tx,
    } = telegram;

    let mut call_audio_tx: Option<Sender<telegram::AudioFrame>> = None;
    let client_channel_id: Arc<Mutex<Option<ChannelId>>> = Arc::new(Mutex::new(None));

    loop {
        tokio::select! {
            Some(event) = a_event_rx.recv() => {
                match event {
                    asterisk::AsteriskEvent::CallRequested { channel_id, call_num, audio_rx, bridge: _ } => {
                        info!("call requested to user {call_num}");

                        let tg_contact: i64 = match router::evaluate_outbound(call_num as u64, config) {
                            Some(c) => c,
                            None => {
                                warn!("number {call_num} is not a contact");
                                // TODO: give bad user Tone or sound then hangup
                                continue;
                            },
                        };

                        {
                            let mut guard = client_channel_id.lock().await;
                            *guard = Some(channel_id);
                        }

                        let (once_call_audio_tx, once_call_audio_rx) = mpsc::channel(32);
                        call_audio_tx = Some(once_call_audio_tx);

                        let tg_audio_out_tx = tg_audio_out_tx.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_asterisk_outgoing(audio_rx, tg_audio_out_tx, once_call_audio_rx).await {
                                error!("asterisk outgoing task error: {e}");
                            }
                        });

                        tg_cmd_tx.send(telegram::TelegramCommand::Dial { user_id: tg_contact.into() }).await?;
                    },
                    asterisk::AsteriskEvent::CallEnded => {
                        info!("phone hung up, call has ended");
                        call_audio_tx = None;
                    },
                }
            }
            Some(frame) = tg_audio_in_rx.recv() => {
                if let Some(tx) = &call_audio_tx {
                    let _ = tx.send(frame).await;
                }
            }
            Some(event) = tg_event_rx.recv() => {
                match event {
                    TelegramEvent::CallConnected { user_id: _ }=> {
                        info!("Telegram Call connected");

                        let guard = client_channel_id.lock().await;
                        match &*guard  {
                            Some(channel_id) => {
                                a_cmd_tx.send(AsteriskCommand::AcceptCall(channel_id.clone())).await?;
                            },
                            None => error!("Call connected with no asterist channel ID in store"),
                        }
                    }
                    TelegramEvent::CallEnded { reason, .. } => {
                        info!("[telegram] call ended: {reason:?}");
                        let guard = client_channel_id.lock().await;
                        match &*guard  {
                            Some(channel_id) => {
                                a_cmd_tx.send(AsteriskCommand::Hangup(channel_id.clone())).await?;
                            },
                            None => error!("Call connected with no asterist channel ID in store"),
                        }
                        call_audio_tx = None;
                        break;
                    }
                    _ => {} // TODO: more events
                }
            }

            _ = tokio::signal::ctrl_c() => {
                warn!("SIGINT received, shutting down");
                tg_cmd_tx.send(TelegramCommand::Shutdown).await?;
                a_cmd_tx.send(AsteriskCommand::Shutdown).await?;
                break;
            }

        }
    }

    let (asterisk_result, tg_result) = tokio::join!(asterisk_client_task, telegram_client_task,);
    asterisk_result??;
    tg_result?;
    Ok(())
}
