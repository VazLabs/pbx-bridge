use std::fmt;

use asterisk_ari::apis::bridges::models::Bridge;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::AudioSocketHandle;

#[derive(Debug)]
pub enum AsteriskEvent {
    CallRequested {
        channel_id: ChannelId,
        call_num: u8,
        audio_rx: oneshot::Receiver<AudioSocketHandle>,
        bridge: Box<Bridge>,
    },
    CallEnded,
}

#[derive(Debug)]
pub enum AsteriskCommand {
    Hangup(ChannelId),
    Shutdown,
    IncomingCall,
    AcceptCall(ChannelId),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ChannelId(String);

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ChannelId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<ChannelId> for String {
    fn from(value: ChannelId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalMediaId(Uuid);

impl ExternalMediaId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn inner(&self) -> Uuid {
        self.0
    }
}

impl Default for ExternalMediaId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ExternalMediaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for ExternalMediaId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl From<ExternalMediaId> for Uuid {
    fn from(id: ExternalMediaId) -> Uuid {
        id.0
    }
}
