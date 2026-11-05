use config::BridgeConfig;
use telegram::{AudioFrame, TelegramCommand, TelegramEvent};
use tokio::{
    io::AsyncBufReadExt,
    time::{Duration, sleep},
};
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
    let (client, _shutdown) = ferogram::Client::builder()
        .api_id(config.telegram.api_id)
        .api_hash(&config.telegram.api_hash)
        .session(&config.telegram.session_file)
        .connect()
        .await?;

    let (mut handle, actor) = telegram::spawn(client);

    // In the proper bridge, use a OS thread to create a runtime specifically for
    // this Actor
    let local = tokio::task::LocalSet::new();

    local.spawn_local(async move {
        if let Err(e) = actor.run().await {
            error!("[telegram] actor died: {e:#}");
        }
    });

    // harness loop runs inside the LocalSet so it can share the thread,
    // but only touches handle which is pure Send channels
    local
        .run_until(async move {
            let command_tx = handle.cmd_tx.clone();
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

            loop {
                tokio::select! {
                    Ok(Some(line)) = reader.next_line() => {
                        match line.trim().parse().unwrap_or(99u8) {
                            0 => {
                                command_tx.send(TelegramCommand::Shutdown).await?;
                                break;
                            }
                            1 => {
                                info!("Sending a call request to {:?}", test_user);
                                command_tx
                                    .send(telegram::TelegramCommand::Dial { user_id: test_user.into() })
                                    .await?;
                            }
                            2 => {
                                info!("Hanging up call");
                                command_tx.send(telegram::TelegramCommand::Hangup).await?;
                            }
                            3 => {
                                info!("sending test tone");
                                let tx = handle.audio_out_tx.clone();
                                tokio::spawn(async move {
                                    let frames = generate_sine_frames(440.0, 5.0);
                                    for frame in frames {
                                        if tx.send(frame).await.is_err() {
                                            error!{"error sending frame"};
                                            break;
                                        }
                                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                                    }
                                });
                            }
                            _ => info!("unknown command"),
                        }
                    }

                    Some(event) = handle.event_rx.recv() => {
                        debug!("[event] {event:?}");
                        match event {
                            TelegramEvent::CallConnected { user_id }=> {
                                info!("audio channels are live, user_id: {user_id} — start pumping frames");
                            }
                            TelegramEvent::CallEnded { reason, .. } => {
                                info!("call ended: {reason:?}");
                            }
                            _ => {}
                        }
                    }

                    Some(frame) = handle.audio_in_rx.recv() => {
                        trace!("[audio] got {} samples @ {}µs",
                            frame.samples.len(), frame.timestamp_us);

                        let tx = handle.audio_out_tx.clone();
                        echo_audio_back(tx, frame, 1);

                    }
                }
            }

            Ok::<_, anyhow::Error>(())
        })
        .await?;

    Ok(())
}

fn generate_sine_frames(freq: f32, duration_secs: f32) -> Vec<AudioFrame> {
    let sample_rate = 48_000u32;
    let total_samples = (sample_rate as f32 * duration_secs) as usize;
    let frame_size = 480; // 10ms at 48kHz

    (0..total_samples)
        .step_by(frame_size)
        .enumerate()
        .map(|(i, start)| {
            let samples: Vec<i16> = (start..start + frame_size)
                .map(|n| {
                    let t = n as f32 / sample_rate as f32;
                    (f32::sin(2.0 * std::f32::consts::PI * freq * t) * i16::MAX as f32) as i16
                })
                .collect();
            AudioFrame {
                samples,
                timestamp_us: (i as u64 * 10_000), // 10ms per frame in µs
            }
        })
        .collect()
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
