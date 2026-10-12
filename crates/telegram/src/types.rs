pub use tgcalls::TgCallsError;
use thiserror::Error;
use tokio::sync::mpsc;

// TODO: big enum type (large size, consider boxing)
#[derive(Debug, Error)]
pub enum TelegramError {
    #[error("no session found, need to log in")]
    MissingSession,
    #[error("failed to find peer")]
    PeerError,
    #[error("bad PhoneCall enum type")]
    CallError,
}

// These are the outgoing API boundry events, whatever recieves this events should have no understanding
// of any telegram specific implementation, consider the fields of each enum to be part
// of the bridge types, including Media as needed. Maybe implement a trait to allow
// different clients (non telegram) in the future
// sent OUT of telegram crate
#[derive(Debug)]
pub enum TelegramEvent {
    FatalError(anyhow::Error),
    // A call (inbound or outbound) is connected + audio is live
    CallConnected {
        user_id: i64,
    },
    CallEnded {
        user_id: i64,
        reason: Option<String>,
    },
    IncomingCall {
        user_id: i64,
        call_id: i64,
    },
    Ringing {
        user_id: i64,
    },
    Error(String),
}

// These are the incoming API boundry events, communication should be non-telegram bound
// including incoming call data
// these get sent INTO telegram crate
#[derive(Debug)]
pub enum TelegramCommand {
    Dial { user_id: i64 },
    Hangup,
    Shutdown,
}

/// A single chunk of raw PCM audio.
/// 16-bit signed, 48 kHz, mono — matches ntgcalls default output.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Raw PCM samples, i16 little-endian.
    pub samples: Vec<i16>,
    /// Microsecond timestamp from the source clock.
    pub timestamp_us: u64,
}

#[derive(Debug, Clone)]
pub struct UserId(pub i64);

#[derive(Clone)]
pub(crate) enum CallState {
    Idle,
    Incoming(i64),
    Waiting(i64), // call ID
    Connecting(i64),
    Accepted(i64),
}

/// All the channel endpoints the caller needs to hold onto.
pub struct TelegramHandle {
    /// Send commands into the telegram actor.
    pub cmd_tx: mpsc::Sender<TelegramCommand>,
    /// Receive state-change events from the telegram actor.
    pub event_rx: mpsc::Receiver<TelegramEvent>,
    /// Receive inbound audio frames from the active call.
    pub audio_in_rx: mpsc::Receiver<AudioFrame>,
    /// Send outbound audio frames into the active call.
    pub audio_out_tx: mpsc::Sender<AudioFrame>,
}
