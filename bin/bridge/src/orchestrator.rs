use std::{thread, time::Duration};

use anyhow::Result;
use asterisk::AsteriskClient;
use config::BridgeConfig;
use media::{AudioSocketHandle, AudioSocketMessage, JitterBuffer};
use telegram::AudioFrame;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info, warn};

use crate::types::{ActorHandlers, ActorTaskHandlers};

pub async fn run_connections(config: &BridgeConfig) -> Result<(ActorTaskHandlers, ActorHandlers)> {
    let (tg_handler, tg_actor) = telegram::spawn_client(config).await?;

    let tg_thread = thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Telegram runtime"); // TODO: dont panic, signal back for a shutdown

        let local = tokio::task::LocalSet::new();

        rt.block_on(local.run_until(async {
            tokio::task::spawn_local(async move {
                if let Err(e) = tg_actor.run().await {
                    error!("[telegram] actor died: {e:#}");
                }
            })
            .await
            .expect("actor.run panicked"); // TODO: handle differently or bind to a handler if needed
        }))
    });

    let telegram_client_task = tokio::task::spawn_blocking(move || {
        tg_thread.join().expect("telegram thread panicked");
    });

    let asterisk_config = config.asterisk.clone();
    let (mut asterisk_client, asterisk_handler) = AsteriskClient::init(asterisk_config);

    let asterisk_client_task = tokio::spawn(async move {
        let result = asterisk_client.run().await;
        if let Err(e) = &result {
            error!("asterisk client.run() failed: {e}");
        }
        result
    });

    let task_handlers = ActorTaskHandlers {
        asterisk: asterisk_client_task,
        telegram: telegram_client_task,
    };

    let actor_handlers = ActorHandlers::new(asterisk_handler, tg_handler);

    Ok((task_handlers, actor_handlers))
}

pub async fn handle_asterisk_outgoing(
    audio_rx: tokio::sync::oneshot::Receiver<AudioSocketHandle>,
    tg_audio_out_tx: tokio::sync::mpsc::Sender<AudioFrame>,
    mut once_call_audio_rx: tokio::sync::mpsc::Receiver<AudioFrame>,
) -> Result<()> {
    let Ok(mut audio_socket) = audio_rx.await else {
        warn!("AudioSocket never connected");
        return Ok(());
    };
    info!("AudioSocket stream ready, starting media loop");

    let mut from_tg_jb = JitterBuffer::new(20, 200, 20);
    let mut from_asterisk_jb = JitterBuffer::new(40, 200, 10);
    let mut ticker = interval(Duration::from_millis(10));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            Some(msg) = audio_socket.next_message() => {
                match msg {
                    AudioSocketMessage::StreamId(uuid) => info!(%uuid, "AudioSocket handshake"),
                    AudioSocketMessage::Dtmf(c) => info!(%c, "DTMF received"),
                    AudioSocketMessage::Terminate | AudioSocketMessage::Error(_) => break,
                    AudioSocketMessage::Audio(_, frame) => from_asterisk_jb.push_bytes(&frame),
                }
            }
            _ = ticker.tick() => {
                let chunk = from_asterisk_jb.pop_chunk_or_silence();
                let _ = tg_audio_out_tx.send(AudioFrame {
                    samples: chunk,
                    timestamp_us: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_micros() as u64,
                }).await;
            }
            result = once_call_audio_rx.recv() => {
                match result {
                    Some(frame) => {
                        from_tg_jb.on_samples(&frame.samples, |frame| audio_socket.push_audio(media::samples_to_le_bytes(&frame)));
                    },
                    None => break,
                }

            }
        }
    }

    Ok(())
}
