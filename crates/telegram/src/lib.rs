mod call;
mod client;

pub mod types;

pub use client::{TelegramClient, login};
pub use types::{AudioChannelHandler, AudioFrame, TelegramCommand, TelegramEvent};
