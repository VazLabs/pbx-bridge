mod error;
mod loader;
mod types;

pub use error::ConfigError;
pub use loader::load;
pub use types::{AsteriskConfig, BridgeConfig, Contact, Rules, TelegramConfig};
