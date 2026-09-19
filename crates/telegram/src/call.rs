use crate::types::AudioFrame;
use anyhow::Result;
use ferogram::tl;
use ferogram::tl::enums::{PhoneCall, Update};
use ferogram::update::RawUpdate;
use ferogram::{Client, UpdateStream};
use ntgcalls::VideoRotation;
use std::sync::Arc;
use tgcalls::{
    AudioDescription, FrameData, MediaDescription, MediaSource, P2PCall, RTCServer, StreamDevice,
    StreamMode, TgCallsError,
};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, trace, warn};

/// Sample rate and channel count must match what ntgcalls expects.
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u8 = 1;

pub struct ActiveCall {
    call: Arc<Mutex<P2PCall>>,
    client: Client,
    call_id: i64,
    peer: tl::enums::InputPhoneCall,
}

pub enum UpdateOutcome {
    Ignored,
    SignalingDelivered,
    Discarded,
}

impl ActiveCall {
    /// Dial a user and negotiate the call parameters.
    /// Does NOT connect WebRTC yet — call `.connect()` next.
    pub async fn dial(
        client: &Client,
        user_id: i64,
        stream: &mut UpdateStream,
    ) -> Result<(Self, Vec<RTCServer>, Vec<String>)> {
        let mut call = P2PCall::new(client.clone(), user_id);

        match call.request(false, stream).await {
            Ok((servers, versions)) => {
                let call_id = match call.call_id() {
                    Some(call_id) => call_id,
                    None => return Err(anyhow::anyhow!("failed to get call ID")),
                };

                let peer = match call.call_peer() {
                    Some(peer) => peer,
                    None => return Err(anyhow::anyhow!("failed to get call Peer")),
                };

                Ok((
                    Self {
                        call: Arc::new(Mutex::new(call)),
                        client: client.clone(),
                        call_id,
                        peer,
                    },
                    servers,
                    versions,
                ))
            }
            Err(e) => {
                call.end().await;
                Err(e.into())
            }
        }
    }

    /// Establish the WebRTC transport. Returns the signaling channels
    /// that must be passed to `run_signaling`.
    pub async fn connect(
        &mut self,
        servers: &[RTCServer],
        versions: &[String],
    ) -> Result<(mpsc::Receiver<Vec<u8>>, mpsc::Receiver<bool>)> {
        match self
            .call
            .lock()
            .await
            .connect(servers, versions, true)
            .await
        {
            Ok((sig_rx, conn_rx)) => Ok((sig_rx, conn_rx)),
            Err(e) => {
                self.call.lock().await.end().await;
                Err(e.into())
            }
        }
    }

    pub async fn activate_audio(
        &mut self,
        // audio_in_tx: mpsc::Sender<AudioFrame>,
        mut audio_out_rx: mpsc::Receiver<AudioFrame>,
    ) -> Result<()> {
        // Outbound: audio_out_rx → send_external_frame
        let call = Arc::clone(&self.call);
        tokio::task::spawn_local(async move {
            debug!("outbound pump: started");
            while let Some(frame) = audio_out_rx.recv().await {
                let data = samples_to_bytes(&frame.samples);
                let frame_data = FrameData {
                    absolute_capture_timestamp_ms: (frame.timestamp_us / 1000) as i64,
                    rotation: VideoRotation::VideoRotation0,
                    width: 0,
                    height: 0,
                };
                let result = {
                    let mut call = call.lock().await;
                    call.send_external_frame(StreamDevice::Microphone, &data, &frame_data)
                        .await
                };
                match result {
                    Ok(()) => trace!("outbound pump: sent {} bytes", data.len()),
                    Err(e) => {
                        error!("outbound pump: send_external_frame failed: {e}");
                        break;
                    }
                }
            }
            debug!("outbound pump: exiting");
        });

        debug!("activate_audio: done");
        Ok(())
    }

    pub async fn prepare_media(&mut self, audio_in_tx: mpsc::Sender<AudioFrame>) -> Result<()> {
        debug!("activate_audio: registering on_frames callback");
        // Inbound: ntgcalls → audio_in_tx
        {
            let mut call = self.call.lock().await;
            call.on_frames(move |user_id, mode, device, frames| {
                trace!(
                    "on_frames fired: user_id={} mode={:?} device={:?} frames={}",
                    user_id,
                    mode,
                    device,
                    frames.len()
                );
                // // no filtering yet
                if mode != StreamMode::Playback {
                    return;
                }
                for frame in frames {
                    let audio_frame = AudioFrame {
                        samples: bytes_to_samples(&frame.data),
                        timestamp_us: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_micros() as u64,
                    };
                    if audio_in_tx.try_send(audio_frame).is_err() {
                        warn!("on_frames: audio_in_tx full or dropped");
                    }
                }
            });
        }

        debug!("activate_audio: setting media to External");
        let media: MediaDescription = MediaDescription {
            microphone: Some(AudioDescription {
                media_source: MediaSource::External,
                sample_rate: SAMPLE_RATE,
                channel_count: CHANNELS,
                input: String::new(), // not used for External
                keep_open: false,
            }),
            speaker: Some(AudioDescription {
                media_source: MediaSource::External,
                sample_rate: SAMPLE_RATE,
                channel_count: CHANNELS,
                input: String::new(),
                keep_open: false,
            }),
            camera: None,
            screen: None,
        };

        {
            let call = self.call.lock().await;
            call.set_media(StreamMode::Capture, &media).await?;
            debug!("activate_audio: set_media Capture OK");
            call.set_media(StreamMode::Playback, &media).await?;
            debug!("activate_audio: set_media Playback OK");
        }

        debug!("activate_audio: on_frames registered, starting outbound pump");
        Ok(())
    }

    pub async fn run_signaling(
        &mut self,
        sig_out_rx: &mut mpsc::Receiver<Vec<u8>>,
        conn_rx: &mut mpsc::Receiver<bool>,
        stream: &mut UpdateStream,
    ) -> Result<bool> {
        Ok(self
            .call
            .lock()
            .await
            .run_signaling(sig_out_rx, conn_rx, stream)
            .await?)
    }

    pub async fn end(&mut self) {
        self.call.lock().await.end().await;
    }

    pub async fn send_signaling(&self, bytes: &[u8]) -> Result<(), TgCallsError> {
        tgcalls::send_p2p_signaling(&self.client, self.peer.clone(), bytes).await?;
        Ok(())
    }

    pub async fn handle_update(&self, upd: RawUpdate) -> Result<UpdateOutcome> {
        match upd.constructor_id {
            0x2661bf09 => {
                if let Update::PhoneCallSignalingData(sd) = upd.inner {
                    if sd.phone_call_id == self.call_id {
                        self.call
                            .lock()
                            .await
                            .receive_signaling_data(&sd.data)
                            .await?;
                        return Ok(UpdateOutcome::SignalingDelivered);
                    }
                }
            }
            0xab0f6b1e => {
                if let Update::PhoneCall(pc) = upd.inner {
                    if let PhoneCall::Discarded(d) = pc.phone_call {
                        if d.id == self.call_id {
                            return Ok(UpdateOutcome::Discarded);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(UpdateOutcome::Ignored)
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn samples_to_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn bytes_to_samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}
