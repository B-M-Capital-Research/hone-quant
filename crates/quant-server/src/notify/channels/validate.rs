//! Configuration checks, run when a channel is saved and again before every delivery.
//!
//! Messages name the offending field but never echo a secret value: they are shown in the UI and
//! written to logs.

use lettre::message::Mailbox;

use super::secrets::is_masked;
use super::{ChannelConfig, ChannelError};

/// Checks that `config` is complete and safe to use.
///
/// - Required fields are non-blank, and no secret still holds a mask from
///   [`redacted`](super::redacted) (one that [`merge_secrets`](super::merge_secrets) could not
///   restore).
/// - Webhook URLs use `https://`; plain `http://` is accepted only for `localhost`, `127.0.0.1`
///   and `[::1]`, since the URL itself is a credential on most platforms.
/// - Email needs a host, a non-zero port, and a valid sender and at least one valid recipient.
pub fn validate(config: &ChannelConfig) -> Result<(), ChannelError> {
    match config {
        ChannelConfig::Telegram { bot_token, chat_id } => {
            check_secret("Telegram bot token", bot_token)?;
            // The token becomes a URL path segment.
            let breaks_url = |c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#');
            if bot_token.trim().contains(breaks_url) {
                return Err(invalid("Telegram bot token contains invalid characters"));
            }
            check_required("Telegram chat ID", chat_id)
        }
        ChannelConfig::Feishu {
            webhook_url,
            secret,
        } => {
            check_url("Feishu webhook URL", webhook_url)?;
            check_optional_secret("Feishu signing secret", secret.as_deref())
        }
        ChannelConfig::Wecom { webhook_url } => check_url("WeCom webhook URL", webhook_url),
        ChannelConfig::Slack { webhook_url } => check_url("Slack webhook URL", webhook_url),
        ChannelConfig::Discord { webhook_url } => check_url("Discord webhook URL", webhook_url),
        ChannelConfig::Webhook { url, secret } => {
            check_url("Webhook URL", url)?;
            check_optional_secret("Webhook signing secret", secret.as_deref())
        }
        ChannelConfig::Email {
            host,
            port,
            password,
            from,
            to,
            ..
        } => {
            check_required("SMTP host", host)?;
            if *port == 0 {
                return Err(invalid("SMTP port must be between 1 and 65535"));
            }
            check_optional_secret("SMTP password", password.as_deref())?;
            check_address("Sender", from)?;
            if to.is_empty() {
                return Err(invalid("At least one recipient is required"));
            }
            to.iter()
                .try_for_each(|recipient| check_address("Recipient", recipient))
        }
    }
}

fn invalid(message: impl Into<String>) -> ChannelError {
    ChannelError::Config(message.into())
}

fn check_required(field: &str, value: &str) -> Result<(), ChannelError> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{field} is required")));
    }
    Ok(())
}

fn check_unmasked(field: &str, value: &str) -> Result<(), ChannelError> {
    if is_masked(value) {
        return Err(invalid(format!("{field} is masked; enter it again")));
    }
    Ok(())
}

fn check_secret(field: &str, value: &str) -> Result<(), ChannelError> {
    check_required(field, value)?;
    check_unmasked(field, value)
}

fn check_optional_secret(field: &str, value: Option<&str>) -> Result<(), ChannelError> {
    value.map_or(Ok(()), |value| check_unmasked(field, value))
}

fn check_url(field: &str, value: &str) -> Result<(), ChannelError> {
    check_secret(field, value)?;
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| invalid(format!("{field} is not a valid URL")))?;
    match url.scheme() {
        "https" => Ok(()),
        "http" if matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) => Ok(()),
        _ => Err(invalid(format!("{field} must start with https://"))),
    }
}

/// `addr@example.com` or `Name <addr@example.com>`.
fn check_address(role: &str, value: &str) -> Result<(), ChannelError> {
    let value = value.trim();
    if !value.contains('@') || value.parse::<Mailbox>().is_err() {
        return Err(invalid(format!(
            "{role} address {value:?} is not a valid email address"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testkit::samples;
    use super::super::{EmailTls, redacted};
    use super::*;

    fn rejection(config: &ChannelConfig) -> String {
        match validate(config) {
            Err(ChannelError::Config(message)) => message,
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    fn email(from: &str, to: &[&str], port: u16) -> ChannelConfig {
        ChannelConfig::Email {
            host: "smtp.example.com".into(),
            port,
            username: None,
            password: None,
            from: from.into(),
            to: to.iter().map(|s| s.to_string()).collect(),
            tls: EmailTls::Implicit,
        }
    }

    #[test]
    fn sample_configs_are_valid() {
        for config in samples() {
            validate(&config).unwrap();
        }
    }

    #[test]
    fn required_fields_must_be_present() {
        let telegram = |bot_token: &str, chat_id: &str| ChannelConfig::Telegram {
            bot_token: bot_token.into(),
            chat_id: chat_id.into(),
        };
        assert!(rejection(&telegram("", "1")).contains("bot token is required"));
        assert!(rejection(&telegram("123:abc", " ")).contains("chat ID is required"));
        let slack = ChannelConfig::Slack {
            webhook_url: "  ".into(),
        };
        assert!(rejection(&slack).contains("Slack webhook URL is required"));
        let mut blank_host = email("a@example.com", &["b@example.com"], 465);
        if let ChannelConfig::Email { host, .. } = &mut blank_host {
            host.clear();
        }
        assert!(rejection(&blank_host).contains("SMTP host is required"));
    }

    #[test]
    fn telegram_token_must_fit_in_a_url_path() {
        for token in ["123:abc/def", "123:abc def", "123:abc?x", "123:abc#x"] {
            let config = ChannelConfig::Telegram {
                bot_token: token.into(),
                chat_id: "1".into(),
            };
            assert!(rejection(&config).contains("invalid characters"), "{token}");
        }
    }

    #[test]
    fn webhook_urls_need_https_except_on_loopback() {
        let accepted = [
            "https://hooks.slack.com/services/T/B/x",
            "https://open.feishu.cn/open-apis/bot/v2/hook/x",
            "http://localhost:8080/hook",
            "http://127.0.0.1:9000/hook",
            "http://[::1]:9000/hook",
            " https://hooks.example.com/padded ",
        ];
        for url in accepted {
            let config = ChannelConfig::Webhook {
                url: url.into(),
                secret: None,
            };
            assert!(validate(&config).is_ok(), "{url} should be accepted");
        }
        let rejected = [
            "http://hooks.slack.com/services/T/B/x",
            "http://localhost.example.com/hook",
            "http://10.0.0.5/hook",
            "ftp://example.com/hook",
            "hooks.slack.com/services/T/B/x",
            "not a url",
        ];
        for url in rejected {
            let config = ChannelConfig::Discord {
                webhook_url: url.into(),
            };
            assert!(validate(&config).is_err(), "{url} should be rejected");
        }
    }

    #[test]
    fn email_needs_port_sender_and_recipients() {
        validate(&email(
            "hone-quant <alerts@example.com>",
            &["a@example.com"],
            465,
        ))
        .unwrap();
        assert!(rejection(&email("a@example.com", &["b@example.com"], 0)).contains("port"));
        assert!(rejection(&email("alerts", &["b@example.com"], 25)).contains("Sender"));
        assert!(rejection(&email("a@example.com", &[], 25)).contains("recipient"));
        assert!(
            rejection(&email("a@example.com", &["b@example.com", "ops"], 25)).contains("\"ops\"")
        );
        assert!(rejection(&email("a@example.com", &["a b@c d"], 25)).contains("Recipient"));
    }

    #[test]
    fn masked_secrets_are_rejected() {
        for config in samples() {
            let message = rejection(&redacted(&config));
            assert!(message.contains("masked"), "{message}");
        }
    }

    #[test]
    fn errors_do_not_echo_secrets() {
        let config = ChannelConfig::Slack {
            webhook_url: "http://hooks.slack.com/services/T1/B2/plaintext-secret".into(),
        };
        let message = rejection(&config);
        assert!(message.contains("https://"));
        assert!(!message.contains("plaintext-secret") && !message.contains("hooks.slack.com"));
    }
}
