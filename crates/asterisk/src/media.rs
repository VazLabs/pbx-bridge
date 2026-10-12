use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        TcpListener, TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::{
        mpsc::{self, error::TryRecvError},
        oneshot,
    },
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

pub const FRAME_BYTES: usize = 320; // 20 ms of slin mono 16-bit
const FRAME_PERIOD: Duration = Duration::from_millis(20);
const QUEUE_DEPTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Terminate,
    StreamId,
    Dtmf,
    Audio(SampleRate),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    Hz8000,
    Hz12000,
    Hz16000,
    Hz24000,
    Hz32000,
    Hz44100,
    Hz48000,
    Hz96000,
    Hz192000,
}

impl SampleRate {
    pub fn as_hz(&self) -> u32 {
        match self {
            Self::Hz8000 => 8_000,
            Self::Hz12000 => 12_000,
            Self::Hz16000 => 16_000,
            Self::Hz24000 => 24_000,
            Self::Hz32000 => 32_000,
            Self::Hz44100 => 44_100,
            Self::Hz48000 => 48_000,
            Self::Hz96000 => 96_000,
            Self::Hz192000 => 192_000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unknown message kind byte: {0:#04x}")]
    UnknownKind(u8),
    #[error("invalid UUID payload: expected 16 bytes, got {0}")]
    InvalidUuid(usize),
    #[error("invalid DTMF payload: expected 1 ASCII byte, got {0:?}")]
    InvalidDtmf(Vec<u8>),
}

impl TryFrom<u8> for MessageKind {
    type Error = ParseError;

    fn try_from(byte: u8) -> Result<Self, ParseError> {
        match byte {
            0x00 => Ok(Self::Terminate),
            0x01 => Ok(Self::StreamId),
            0x03 => Ok(Self::Dtmf),
            0x10 => Ok(Self::Audio(SampleRate::Hz8000)),
            0x11 => Ok(Self::Audio(SampleRate::Hz12000)),
            0x12 => Ok(Self::Audio(SampleRate::Hz16000)),
            0x13 => Ok(Self::Audio(SampleRate::Hz24000)),
            0x14 => Ok(Self::Audio(SampleRate::Hz32000)),
            0x15 => Ok(Self::Audio(SampleRate::Hz44100)),
            0x16 => Ok(Self::Audio(SampleRate::Hz48000)),
            0x17 => Ok(Self::Audio(SampleRate::Hz96000)),
            0x18 => Ok(Self::Audio(SampleRate::Hz192000)),
            0xff => Ok(Self::Error),
            unknown => Err(ParseError::UnknownKind(unknown)),
        }
    }
}

impl From<MessageKind> for u8 {
    fn from(kind: MessageKind) -> u8 {
        match kind {
            MessageKind::Terminate => 0x00,
            MessageKind::StreamId => 0x01,
            MessageKind::Dtmf => 0x03,
            MessageKind::Audio(rate) => match rate {
                SampleRate::Hz8000 => 0x10,
                SampleRate::Hz12000 => 0x11,
                SampleRate::Hz16000 => 0x12,
                SampleRate::Hz24000 => 0x13,
                SampleRate::Hz32000 => 0x14,
                SampleRate::Hz44100 => 0x15,
                SampleRate::Hz48000 => 0x16,
                SampleRate::Hz96000 => 0x17,
                SampleRate::Hz192000 => 0x18,
            },
            MessageKind::Error => 0xff,
        }
    }
}

// https://docs.asterisk.org/Configuration/Channel-Drivers/AudioSocket/

#[derive(Debug)]
pub enum AudioSocketMessage {
    Terminate,
    StreamId(Uuid),
    Dtmf(char),
    Audio(SampleRate, Vec<u8>),
    Error(Option<u8>),
}

impl AudioSocketMessage {
    pub fn parse(kind: u8, payload: Vec<u8>) -> Result<Self, ParseError> {
        let kind = MessageKind::try_from(kind)?;

        match kind {
            MessageKind::Terminate => Ok(Self::Terminate),

            MessageKind::StreamId => {
                let bytes: [u8; 16] = payload
                    .try_into()
                    .map_err(|v: Vec<u8>| ParseError::InvalidUuid(v.len()))?;
                Ok(Self::StreamId(Uuid::from_bytes(bytes)))
            }

            MessageKind::Dtmf => match payload.as_slice() {
                [b] if b.is_ascii() => Ok(Self::Dtmf(*b as char)),
                _ => Err(ParseError::InvalidDtmf(payload)),
            },

            MessageKind::Audio(rate) => Ok(Self::Audio(rate, payload)),

            MessageKind::Error => Ok(Self::Error(payload.first().copied())),
        }
    }
}

/// Owns an accepted AudioSocket connection. Reads inbound messages and
/// sends one paced 20 ms frame per tick, using silence when no audio is queued.
#[derive(Debug)]
pub struct AudioSocketHandle {
    messages: mpsc::Receiver<AudioSocketMessage>,
    audio_in: mpsc::Sender<Vec<u8>>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl AudioSocketHandle {
    fn new(stream: TcpStream) -> Self {
        let (rd, wr) = stream.into_split();
        let (msg_tx, msg_rx) = mpsc::channel(QUEUE_DEPTH);
        let (audio_tx, audio_rx) = mpsc::channel(QUEUE_DEPTH);

        Self {
            messages: msg_rx,
            audio_in: audio_tx,
            reader: tokio::spawn(read_loop(rd, msg_tx)),
            writer: tokio::spawn(write_loop(wr, audio_rx)),
        }
    }

    /// Next message from Asterisk, or `None` once the connection has closed.
    pub async fn next_message(&mut self) -> Option<AudioSocketMessage> {
        self.messages.recv().await
    }

    pub fn push_audio(&self, frame: Vec<u8>) {
        if frame.len() != FRAME_BYTES {
            warn!(bytes = frame.len(), "wrong audio frame size, dropping");
            return;
        }
        if self.audio_in.try_send(frame).is_err() {
            debug!("audio queue full, dropping frame");
        }
    }
}

impl Drop for AudioSocketHandle {
    fn drop(&mut self) {
        debug!("dropping AudioSocketHandle");
        self.reader.abort();
        self.writer.abort();
    }
}

async fn read_loop(mut rd: OwnedReadHalf, messages: mpsc::Sender<AudioSocketMessage>) {
    loop {
        let msg = match read_message(&mut rd).await {
            Ok(AudioSocketMessage::StreamId(uuid)) => {
                info!(%uuid, "AudioSocket handshake");
                continue;
            }
            Ok(msg) => msg,
            Err(e) => {
                if is_eof(&e) {
                    info!("AudioSocket closed by Asterisk");
                } else {
                    error!("while parsing audio socket message: {e}");
                }
                break;
            }
        };

        let stop = matches!(
            msg,
            AudioSocketMessage::Terminate | AudioSocketMessage::Error(_)
        );
        if messages.send(msg).await.is_err() || stop {
            break;
        }
    }
}

async fn write_loop(mut wr: OwnedWriteHalf, mut audio_in: mpsc::Receiver<Vec<u8>>) {
    let mut ticker = interval(FRAME_PERIOD);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;

        let frame = match audio_in.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Empty) => vec![0u8; FRAME_BYTES],
            Err(TryRecvError::Disconnected) => return,
        };

        if let Err(e) = write_message(
            &mut wr,
            &AudioSocketMessage::Audio(SampleRate::Hz8000, frame),
        )
        .await
        {
            error!("AudioSocket write failed, stopping writer: {e}");
            return;
        }
    }
}

async fn read_message(rd: &mut OwnedReadHalf) -> Result<AudioSocketMessage> {
    let kind = rd.read_u8().await?;
    let length = rd.read_u16().await? as usize;

    let mut payload = vec![0u8; length];
    rd.read_exact(&mut payload).await?;

    AudioSocketMessage::parse(kind, payload).map_err(anyhow::Error::from)
}

async fn write_message(wr: &mut OwnedWriteHalf, message: &AudioSocketMessage) -> Result<()> {
    match message {
        AudioSocketMessage::Terminate => {
            wr.write_u8(MessageKind::Terminate.into()).await?;
            wr.write_u16(0).await?;
        }
        AudioSocketMessage::StreamId(uuid) => {
            wr.write_u8(MessageKind::StreamId.into()).await?;
            wr.write_u16(16).await?;
            wr.write_all(uuid.as_bytes()).await?;
        }
        AudioSocketMessage::Audio(rate, payload) => {
            let mut buf = Vec::with_capacity(3 + payload.len());
            buf.push(MessageKind::Audio(*rate).into());
            buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            buf.extend_from_slice(payload);
            wr.write_all(&buf).await?;
        }
        AudioSocketMessage::Dtmf(_) | AudioSocketMessage::Error(_) => {
            return Err(anyhow!("unexpected outgoing AudioSocketMessage kind"));
        }
    }
    Ok(())
}

fn is_eof(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof)
}

/// Binds the AudioSocket port and resolves to a ready handle once Asterisk connects.
/// This is the only way to obtain an AudioSocketHandle
pub async fn start_audiosocket_listener() -> Result<(u16, oneshot::Receiver<AudioSocketHandle>)> {
    let listener = TcpListener::bind("0.0.0.0:40000").await?; // TODO: set port to 0 for random port
    let port = listener.local_addr()?.port();

    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("AudioSocket connection established from {addr}");
                if let Err(e) = stream.set_nodelay(true) {
                    error!("failed to set TCP_NODELAY: {e}");
                }
                let _ = tx.send(AudioSocketHandle::new(stream));
            }
            Err(e) => {
                error!("AudioSocket accept failed: {e}");
            }
        }
    });

    Ok((port, rx))
}
