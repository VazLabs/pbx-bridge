use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct BridgeConfig {
    pub asterisk: AsteriskConfig,
    pub telegram: TelegramConfig,
    pub rules: Rules,
    #[serde(default)]
    pub contacts: Vec<Contact>,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub verbose: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AsteriskConfig {
    pub ari_url: String,
    pub ari_user: String,
    pub ari_password: String,
    pub ari_app_name: String,
    pub rtp_port: u16,
    pub audio_format: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramConfig {
    pub session_file: String,
    pub api_id: i32,
    pub api_hash: String,
    pub phone_num: String,
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
    pub telegram_id: String,
    pub dial_num: u64,
}

fn default_log_level() -> String {
    String::from("info")
}
