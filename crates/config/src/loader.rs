use crate::{BridgeConfig, ConfigError};
use std::path::Path;
use tracing::debug;
use tracing_subscriber::EnvFilter;

pub fn load(path: impl AsRef<Path>) -> Result<BridgeConfig, ConfigError> {
    let path = path.as_ref();

    debug!("loading config from path: {:?}", path);
    let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })?;

    let config: BridgeConfig = toml::from_str(&contents)?;

    validate(&config)?;

    Ok(config)
}

fn validate(config: &BridgeConfig) -> Result<(), ConfigError> {
    if config.asterisk.ari_url.is_empty() {
        return Err(ConfigError::Validation(
            "asterisk.ari_url must not be empty".into(),
        ));
    }

    if config.asterisk.ari_app_name.is_empty() {
        return Err(ConfigError::Validation(
            "asterisk.ari_app_name must not be empty".into(),
        ));
    }

    if config.asterisk.rtp_port < 1024 {
        return Err(ConfigError::Validation(format!(
            "asterisk.rtp_port {} is a privileged port, use 1024 or above",
            config.asterisk.rtp_port
        )));
    }

    if config.rules.whitelist_only && config.contacts.is_empty() {
        return Err(ConfigError::Validation(
            "rules.whitelist_only is true but no contacts are defined — all calls would be rejected".into(),
        ));
    }

    let mut seen_ids = std::collections::HashSet::new();
    for contact in &config.contacts {
        if !seen_ids.insert(contact.telegram_id.clone()) {
            return Err(ConfigError::Validation(format!(
                "Duplicate telegram_id {} in contacts",
                contact.telegram_id
            )));
        }
    }

    Ok(())
}

// TODO: add builder pattern to allow for better configuration
// setup default tracing
pub async fn config_tracing() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let log_level = match load("bridge_config.toml") {
        Ok(config) => config.log_level,
        Err(e) => {
            tracing::error!(error = %e, "failed to load bridge config, defaulting to debug");
            String::from("debug")
        }
    };

    let filter = tracing_subscriber::filter::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()?;

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn valid_toml() -> &'static str {
        r#"
        [asterisk]
        ari_url      = "ws://localhost:8088/ari/events"
        ari_user     = "asterisk"
        ari_password = "secret"
        ari_app_name = "my_app"
        rtp_port     = 10000
        audio_format = "slin"

        [telegram]
        session_file = "userbot.session"
        api_id = 1234567
        api_hash = "long_hash"
        phone_num = "+11234567890"

        [rules]
        whitelist_only    = true
        default_extension = "101"

        [[contacts]]
        name        = "Alice"
        telegram_id = "123456789"
        dial_num    = 1
        "#
    }

    #[test]
    fn parses_valid_config() {
        let config: BridgeConfig = toml::from_str(valid_toml()).unwrap();
        assert_eq!(config.contacts.len(), 1);
        assert_eq!(config.contacts[0].name, "Alice");
        assert_eq!(config.asterisk.rtp_port, 10000);
    }

    #[test]
    fn rejects_privileged_rtp_port() {
        let toml = valid_toml().replace("rtp_port     = 10000", "rtp_port = 80");
        let config: BridgeConfig = toml::from_str(&toml).unwrap();
        let err = validate(&config).unwrap_err();
        assert!(err.to_string().contains("privileged port"));
    }

    #[test]
    fn rejects_whitelist_with_no_contacts() {
        let toml = r#"
        [asterisk]
        ari_url      = "ws://localhost:8088/ari/events"
        ari_user     = "asterisk"
        ari_password = "secret"
        ari_app_name = "my_app"
        rtp_port     = 10000
        audio_format = "slin"

        [telegram]
        session_file = "userbot.session"
        api_id = 1234567
        api_hash = "long_hash"
        phone_num = "+11234567890"

        [rules]
        whitelist_only = true
        "#;
        let config: BridgeConfig = toml::from_str(toml).unwrap();
        let err = validate(&config).unwrap_err();
        assert!(err.to_string().contains("all calls would be rejected"));
    }

    #[test]
    fn rejects_duplicate_telegram_ids() {
        let toml = r#"
        [asterisk]
        ari_url      = "ws://localhost:8088/ari/events"
        ari_user     = "asterisk"
        ari_password = "secret"
        ari_app_name = "my_app"
        rtp_port     = 10000
        audio_format = "slin"

        [telegram]
        session_file = "userbot.session"
        api_id = 1234567
        api_hash = "long_hash"
        phone_num = "+11234567890"

        [rules]
        whitelist_only = false

        [[contacts]]
        name        = "Alice"
        telegram_id = "123456789"
        dial_num    = 1

        [[contacts]]
        name        = "Alice Duplicate"
        telegram_id = "123456789"
        dial_num    = 1
        "#;
        let config: BridgeConfig = toml::from_str(toml).unwrap();
        let err = validate(&config).unwrap_err();
        assert!(err.to_string().contains("Duplicate telegram_id"));
    }
}
