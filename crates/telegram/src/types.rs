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
        user_id: UserId,
    },
    CallEnded {
        user_id: UserId,
        reason: Option<String>,
    },
    IncomingCall {
        user_id: UserId,
        call_id: CallId,
    },
    Ringing {
        user_id: UserId,
    },
    Error(String),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct UserId(i64);

impl UserId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    pub fn inner(&self) -> i64 {
        self.0
    }
}

impl From<i64> for UserId {
    fn from(value: i64) -> Self {
        UserId(value)
    }
}

impl From<UserId> for i64 {
    fn from(value: UserId) -> Self {
        value.0
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct CallId(i64);

impl CallId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    pub fn inner(&self) -> i64 {
        self.0
    }
}
impl From<i64> for CallId {
    fn from(value: i64) -> Self {
        CallId(value)
    }
}

impl From<CallId> for i64 {
    fn from(value: CallId) -> Self {
        value.0
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
// These are the incoming API boundry events, communication should be non-telegram bound
// including incoming call data
// these get sent INTO telegram crate
#[derive(Debug)]
pub enum TelegramCommand {
    Dial { user_id: UserId },
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
