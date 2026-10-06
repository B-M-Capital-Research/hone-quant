//! Outbound notification channels: one message, one configured destination, one bounded attempt.
//!
//! Each channel speaks its platform's native rich format — Telegram HTML, a Feishu `post`, WeCom
//! markdown, Slack blocks, a Discord embed, multipart email — rather than a lowest-common-
//! denominator text blob, because these alerts are read on phones where a bold title, a severity
//! cue and a one-tap link are the point. Every platform rejects an over-long message outright, so
//! each limit is enforced here: a truncated alert beats a lost one.
//!
//! Secrets — bot tokens, signing secrets, SMTP passwords, and webhook URLs, which *are* the
//! credential on Feishu, WeCom, Slack and Discord — stay inside this module: [`redacted`] masks
//! them for API responses and logs, [`merge_secrets`] lets the settings UI send masked values
//! back unchanged, and every [`ChannelError`] is scrubbed of them before it is returned.
//!
//! Delivery is a single attempt bounded by [`REQUEST_TIMEOUT`]. Retry and back-off belong to the
//! caller, which knows whether a message is still worth sending.

mod discord;
mod email;
mod feishu;
mod format;
mod secrets;
mod slack;
mod telegram;
mod transport;
mod validate;
mod webhook;
mod wecom;

#[cfg(test)]
mod testkit;

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use secrets::{MASK, merge_secrets, redacted};
pub use validate::validate;

/// Upper bound on one HTTP request, from connecting to the last byte of the reply. SMTP applies
/// it to each command and bounds the whole session separately.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EmailTls {
    /// Plain connection upgraded with STARTTLS, which is required (usually port 587).
    #[default]
    StartTls,
    /// TLS from the first byte (usually port 465).
    Implicit,
    /// No encryption: only for a relay on a trusted network.
    None,
}

/// One configured delivery channel. Every string marked "secret" must be masked by `redacted`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelConfig {
    /// secret: `bot_token`.
    Telegram { bot_token: String, chat_id: String },
    /// Feishu / Lark custom bot. secret: `webhook_url`, `secret` (optional signing secret).
    Feishu {
        webhook_url: String,
        #[serde(default)]
        secret: Option<String>,
    },
    /// WeCom (企业微信) group bot. secret: `webhook_url`.
    Wecom { webhook_url: String },
    /// Slack incoming webhook. secret: `webhook_url`.
    Slack { webhook_url: String },
    /// Discord webhook. secret: `webhook_url`.
    Discord { webhook_url: String },
    /// Generic JSON webhook, optionally HMAC-SHA256 signed. secret: `secret`.
    Webhook {
        url: String,
        #[serde(default)]
        secret: Option<String>,
    },
    /// SMTP email. secret: `password`.
    Email {
        host: String,
        port: u16,
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
        from: String,
        to: Vec<String>,
        #[serde(default)]
        tls: EmailTls,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Success,
    Warning,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Success => "success",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }

    /// Leading cue where the platform has no colour of its own (Telegram, Slack).
    fn emoji(self) -> &'static str {
        match self {
            Severity::Info => "ℹ️",
            Severity::Success => "✅",
            Severity::Warning => "⚠️",
            Severity::Critical => "🚨",
        }
    }

    /// Accent colour as `0xRRGGBB` (Discord embed bar, email header rule).
    fn color(self) -> u32 {
        match self {
            Severity::Info => 0x5B8DEF,
            Severity::Success => 0x1F7A45,
            Severity::Warning => 0xA37B12,
            Severity::Critical => 0xB94432,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OutboundMessage {
    pub title: String,
    /// Plain text; `\n` separates lines.
    pub body: String,
    pub severity: Severity,
    pub category: String,
    /// Absolute URL into hone-quant, when a public base URL is configured.
    pub link: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl OutboundMessage {
    /// The link, when it is an absolute http(s) URL. Anything else is dropped rather than sent:
    /// Discord and Slack reject the whole message over one malformed URL.
    fn web_link(&self) -> Option<&str> {
        let link = self.link.as_deref()?.trim();
        let absolute = ["https://", "http://"].iter().any(|scheme| {
            link.len() > scheme.len()
                && link
                    .get(..scheme.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
        });
        (absolute && !link.contains(char::is_whitespace)).then_some(link)
    }
}

/// Messages never contain a secret (see the module docs).
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The configuration cannot work as stored; retrying will not help.
    #[error("invalid channel configuration: {0}")]
    Config(String),
    /// The platform was unreachable, timed out or rejected the message; may be transient.
    #[error("delivery failed: {0}")]
    Delivery(String),
}

/// Base URLs of hosted APIs that are not part of a channel's configuration. Only tests (and a
/// self-hosted Telegram Bot API server) need anything but the default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Telegram Bot API root; messages go to `{telegram_api_base}/bot{token}/sendMessage`.
    pub telegram_api_base: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            telegram_api_base: "https://api.telegram.org".to_string(),
        }
    }
}

/// The configuration's serialized `kind` tag, e.g. `"telegram"`.
pub fn kind(config: &ChannelConfig) -> &'static str {
    match config {
        ChannelConfig::Telegram { .. } => "telegram",
        ChannelConfig::Feishu { .. } => "feishu",
        ChannelConfig::Wecom { .. } => "wecom",
        ChannelConfig::Slack { .. } => "slack",
        ChannelConfig::Discord { .. } => "discord",
        ChannelConfig::Webhook { .. } => "webhook",
        ChannelConfig::Email { .. } => "email",
    }
}

/// Delivers `message` through `config` using the public platform endpoints.
pub async fn send(
    http: &reqwest::Client,
    config: &ChannelConfig,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    send_with(http, config, message, &Endpoints::default()).await
}

/// [`send`] against explicit [`Endpoints`].
///
/// The configuration is validated first, so a stored channel that predates a rule (say, an
/// `http://` webhook) fails loudly instead of sending its credential in clear text.
pub async fn send_with(
    http: &reqwest::Client,
    config: &ChannelConfig,
    message: &OutboundMessage,
    endpoints: &Endpoints,
) -> Result<(), ChannelError> {
    let result = deliver(http, config, message, endpoints)
        .await
        .map_err(|err| secrets::scrub(err, config));
    match &result {
        Ok(()) => tracing::debug!(channel = kind(config), "notification delivered"),
        Err(err) => tracing::debug!(channel = kind(config), error = %err, "notification failed"),
    }
    result
}

async fn deliver(
    http: &reqwest::Client,
    config: &ChannelConfig,
    message: &OutboundMessage,
    endpoints: &Endpoints,
) -> Result<(), ChannelError> {
    validate(config)?;
    match config {
        ChannelConfig::Telegram { bot_token, chat_id } => {
            let api_base = &endpoints.telegram_api_base;
            telegram::send(http, api_base, bot_token.trim(), chat_id.trim(), message).await
        }
        ChannelConfig::Feishu {
            webhook_url,
            secret,
        } => feishu::send(http, webhook_url.trim(), non_blank(secret), message).await,
        ChannelConfig::Wecom { webhook_url } => {
            wecom::send(http, webhook_url.trim(), message).await
        }
        ChannelConfig::Slack { webhook_url } => {
            slack::send(http, webhook_url.trim(), message).await
        }
        ChannelConfig::Discord { webhook_url } => {
            discord::send(http, webhook_url.trim(), message).await
        }
        ChannelConfig::Webhook { url, secret } => {
            webhook::send(http, url.trim(), non_blank(secret), message).await
        }
        ChannelConfig::Email {
            host,
            port,
            username,
            password,
            from,
            to,
            tls,
        } => {
            let smtp = email::Smtp {
                host: host.trim(),
                port: *port,
                username: username.as_deref().map(str::trim).filter(|u| !u.is_empty()),
                password: password.as_deref(),
                tls: *tls,
            };
            email::send(&smtp, from, to, message).await
        }
    }
}

/// An optional secret, with a blank value meaning "not set" (forms submit empty strings).
fn non_blank(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::testkit::{CaptureServer, client, message, samples};
    use super::*;

    #[test]
    fn kind_matches_the_serialized_tag() {
        let configs = samples();
        for config in &configs {
            let json = serde_json::to_value(config).unwrap();
            assert_eq!(json["kind"], kind(config));
        }
        let kinds: std::collections::BTreeSet<_> = configs.iter().map(kind).collect();
        assert_eq!(kinds.len(), 7, "one sample per kind: {kinds:?}");
    }

    #[test]
    fn optional_fields_default_when_absent() {
        let config: ChannelConfig = serde_json::from_str(
            r#"{"kind":"email","host":"smtp.example.com","port":587,
                "from":"alerts@example.com","to":["ops@example.com"]}"#,
        )
        .unwrap();
        assert!(matches!(
            config,
            ChannelConfig::Email {
                username: None,
                password: None,
                tls: EmailTls::StartTls,
                ..
            }
        ));
        let config: ChannelConfig =
            serde_json::from_str(r#"{"kind":"feishu","webhook_url":"https://open.feishu.cn/x"}"#)
                .unwrap();
        assert!(matches!(config, ChannelConfig::Feishu { secret: None, .. }));
    }

    #[test]
    fn send_future_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let (http, config, message) = (client(), samples().remove(0), message());
        assert_send(&send(&http, &config, &message));
    }

    #[test]
    fn links_must_be_absolute_http_urls() {
        let cases = [
            (
                "https://quant.example.com/plans/1",
                Some("https://quant.example.com/plans/1"),
            ),
            (" HTTP://localhost:8080/x ", Some("HTTP://localhost:8080/x")),
            ("/plans/1", None),
            ("javascript:alert(1)", None),
            ("https://", None),
            ("https://quant.example.com/a b", None),
            ("链接", None),
        ];
        for (link, expected) in cases {
            let message = OutboundMessage {
                link: Some(link.to_string()),
                ..message()
            };
            assert_eq!(message.web_link(), expected, "{link:?}");
        }
        let message = OutboundMessage {
            link: None,
            ..message()
        };
        assert_eq!(message.web_link(), None);
    }

    #[tokio::test]
    async fn invalid_config_is_rejected_before_any_request() {
        let server = CaptureServer::json(200, "{}").await;
        let config = ChannelConfig::Webhook {
            url: server.url("/hook"),
            secret: Some(format!("{MASK}abcd")),
        };
        let err = send(&client(), &config, &message()).await.unwrap_err();
        assert!(matches!(err, ChannelError::Config(_)), "{err}");
        assert_eq!(server.request_count(), 0);
    }

    #[tokio::test]
    async fn unreachable_endpoint_error_omits_the_url() {
        // Bind and drop a listener to get a loopback port that refuses connections.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = ChannelConfig::Slack {
            webhook_url: format!("http://127.0.0.1:{port}/services/T1/B2/unreachable-secret"),
        };
        let err = send(&client(), &config, &message()).await.unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)), "{err}");
        let text = err.to_string();
        assert!(
            !text.contains("unreachable-secret") && !text.contains("/services"),
            "{text}"
        );
    }
}
