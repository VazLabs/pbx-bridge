use config::BridgeConfig;
use telegram::{AudioChannelHandler, AudioFrame, TelegramCommand, TelegramEvent};
use tokio::{
    io::AsyncBufReadExt,
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace};

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
    // main_loop(&bridge_config).await?;
    local_main(&bridge_config).await?;
    Ok(())
}

async fn local_main(config: &BridgeConfig) -> anyhow::Result<()> {
    let (client, mut handler) = telegram::TelegramClient::init(&config.telegram).await?;

    client.spawn_local_client();

    let command_tx = handler.cmd_tx.clone();
    info!("Commands: 0=shutdown, 1=dial test user, 2=hangup active call, 3=send test tone");

    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin).lines();
    let user_name = "TestUser";
    let test_user: i64 = config
        .contacts
        .iter()
        .find(|c| c.name == user_name)
        .ok_or_else(|| anyhow::anyhow!("{user_name} not found in contacts"))?
        .telegram_id
        .parse()?;

    let mut audio_task: Option<(tokio::task::JoinHandle<()>, CancellationToken)> = None;

    loop {
        tokio::select! {
            Ok(Some(line)) = reader.next_line() => {
                match line.trim().parse().unwrap_or(99u8) {
                    0 => {
                        command_tx.send(TelegramCommand::Shutdown).await?;
                        break;
                    }
                    1 => {
                        if audio_task.is_some() {
                            error!("attempting to dial while another call is in progress, aborting this attempt");
                            continue;
                        }

                        info!("Sending a call request to {:?}", test_user);
                        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel(50);
                        let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel(50);
                        let token = CancellationToken::new();

                        let audio_plane_channels = AudioChannelHandler { rx: inbound_rx, tx: outbound_tx.clone() };
                        let handle = tokio::spawn(audio_plane(audio_plane_channels, token.clone()));
                        audio_task = Some((handle, token));

                        let channels = AudioChannelHandler { rx: outbound_rx, tx: inbound_tx };

                        command_tx
                            .send(telegram::TelegramCommand::Dial { user_id: test_user.into(), channels })
                            .await?;
                    }
                    2 => {
                        info!("Hanging up call");
                        command_tx.send(telegram::TelegramCommand::Hangup).await?;
                    }
                    _ => info!("unknown command"),
                }
            }

            Some(event) = handler.event_rx.recv() => {
                debug!("[event] {event:?}");
                match event {
                    TelegramEvent::CallConnected { user_id }=> {
                        info!("audio channels are live, user_id: {user_id} — start pumping frames");
                    }
                    TelegramEvent::CallEnded { reason, .. } => {
                        info!("call ended: {reason:?}");
                        stop_audio_task(&mut audio_task).await;
                    }
                    _ => {} // listen on errors to make sure the audio task stops and we can create a new call
                }
            }
        }
    }

    Ok(())
}

fn echo_audio_back(
    audio_tx_out: tokio::sync::mpsc::Sender<AudioFrame>,
    frame: AudioFrame,
    secs: u64,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(secs)).await;
        if audio_tx_out.send(frame).await.is_err() {
            trace!("error sending echo frame");
        }
    });
}

async fn audio_plane(audio: AudioChannelHandler, token: CancellationToken) {
    let AudioChannelHandler { mut rx, tx } = audio;
    loop {
        tokio::select! {
            biased;

            _ = token.cancelled() => {
                info!("audio plane shutting down");
                break;
            }

            Some(frame) = rx.recv() => {
                trace!("[audio] got {} samples @ {}µs",
                    frame.samples.len(), frame.timestamp_us);

                echo_audio_back(tx.clone(), frame, 1);
            }
        }
    }
}

async fn stop_audio_task(
    audio_task: &mut Option<(tokio::task::JoinHandle<()>, CancellationToken)>,
) {
    if let Some((handle, token)) = audio_task.take() {
        token.cancel();
        let _ = handle.await;
    }
}
