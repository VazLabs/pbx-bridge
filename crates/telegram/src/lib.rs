mod call;
mod client;

pub mod types;

pub use client::{TelegramClient, login, prepare_audio_channels};
pub use types::{AudioChannelHandler, AudioFrame, TelegramCommand, TelegramEvent};
