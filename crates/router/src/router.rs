use config::BridgeConfig;

// given a telegram ID, evaluate if call should be
// accepted or rejected
// if accepted, what CID should be displayed
pub fn evaluate_inbound(telegram_id: &str, config: &BridgeConfig) -> Option<String> {
    config
        .contacts
        .iter()
        .find(|c| c.telegram_id == telegram_id)
        .map(|c| c.name.clone())
}

// given dialed value, evalute if call should be
// allow to continue or hanged up.
// if continues, what Contact/telegramID does it match
// to.
pub fn evaluate_outbound(dialed: u64, config: &BridgeConfig) -> Option<String> {
    config
        .contacts
        .iter()
        .find(|c| c.dial_num == dialed)
        .map(|c| c.telegram_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> BridgeConfig {
        let valid_toml = r#"
        [asterisk]
        ari_url      = "ws://localhost:8088/ari/events"
        ari_user     = "asterisk"
        ari_password = "secret"
        rtp_port     = 10000

        [telegram]
        session_file = "userbot.session"
        api_id = 1234567
        api_hash = "long_hash"
        phone_num = "+11234567890"
        message_buffer = 500

        [rules]
        whitelist_only    = true
        default_extension = "101"

        [[contacts]]
        name        = "Alice"
        telegram_id = "123456789"
        dial_num = 1
        "#;

        toml::from_str(valid_toml).unwrap()
    }

    #[test]
    fn inbound_accepts_whitelisted_contact() {
        let config = valid_config();
        let valid_id = "123456789";
        let result = evaluate_inbound(valid_id, &config);

        assert_ne!(result, None);
    }

    #[test]
    fn inbound_rejects_missing_contact() {
        let config = valid_config();
        let valid_id = "987654321";
        let result = evaluate_inbound(valid_id, &config);

        assert_eq!(result, None);
    }

    #[test]
    fn inbound_returns_caller_id() {
        let config = valid_config();
        let valid_id = "123456789";
        let result = evaluate_inbound(valid_id, &config);

        assert_eq!(result, Some(String::from("Alice")));
    }

    #[test]
    fn outbound_accepts_whitelisted_contact() {
        let config = valid_config();
        let valid_dial_num = 1;
        let result = evaluate_outbound(valid_dial_num, &config);

        assert_ne!(result, None);
    }

    #[test]
    fn outbound_rejects_missing_contact() {
        let config = valid_config();
        let invalid_id = 90;
        let result = evaluate_outbound(invalid_id, &config);

        assert_eq!(result, None);
    }

    #[test]
    fn outbound_returns_telegram_user_id() {
        let config = valid_config();
        let valid_dial_num = 1;
        let result = evaluate_outbound(valid_dial_num, &config);

        assert_eq!(result, Some(String::from("123456789")));
    }
}
