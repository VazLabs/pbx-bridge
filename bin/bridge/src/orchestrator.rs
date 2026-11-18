use std::time::Duration;

use anyhow::Result;
use media::{AudioSocketHandle, AudioSocketMessage, JitterBuffer};
use telegram::{AudioChannelHandler, AudioFrame};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub async fn asterisk_audio_plane(
    audio_rx: tokio::sync::oneshot::Receiver<AudioSocketHandle>,
    channels: telegram::AudioChannelHandler,
    token: CancellationToken,
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

    let AudioChannelHandler { tx, mut rx } = channels;

    loop {
        tokio::select! {
            biased;

            _ = token.cancelled() => {
                info!("asterisk audio plane shutting down");
                break;
            }

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
                let _ = tx.send(AudioFrame {
                    samples: chunk,
                    timestamp_us: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_micros() as u64,
                }).await;
            }
            result = rx.recv() => {
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
