use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct BridgeConfig {
    pub asterisk: AsteriskConfig,
    pub telegram: TelegramConfig,
    pub rules: Rules,
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AsteriskConfig {
    pub ari_url: String,
    pub ari_user: String,
    pub ari_password: String,
    pub rtp_port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramConfig {
    pub session_file: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Rules {
    #[serde(default)]
    pub whitelist_only: bool,
    pub default_extension: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Contact {
    pub name: String,
    pub telegram_id: i64,
    pub extension: String,
}
