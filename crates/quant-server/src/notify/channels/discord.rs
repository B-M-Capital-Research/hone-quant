//! Discord webhook with a single embed: colour-coded by severity, timestamped and linked.

use chrono::SecondsFormat;
use serde_json::{Value, json};

use super::format::{Escape, Unit, fit};
use super::{ChannelError, OutboundMessage, transport};

/// Discord's embed field limits. Together they stay under the 6000-character embed total.
const MAX_TITLE: usize = 256;
const MAX_DESCRIPTION: usize = 4096;
const MAX_FOOTER: usize = 1024;

pub(super) async fn send(
    http: &reqwest::Client,
    webhook_url: &str,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let reply = transport::send(http.post(webhook_url).json(&payload(message))).await?;
    // 204 No Content normally (200 with `?wait=true`); errors carry a JSON `message`.
    if reply.status.is_success() {
        return Ok(());
    }
    let detail = reply.json()["message"].as_str().map(str::to_string);
    Err(reply.rejected("Discord", detail))
}

/// Title and description are omitted when empty: Discord rejects empty embed fields.
fn payload(message: &OutboundMessage) -> Value {
    let footer = format!("hone-quant · {}", message.category);
    let mut embed = json!({
        "color": message.severity.color(),
        "timestamp": message.timestamp.to_rfc3339_opts(SecondsFormat::Secs, true),
        "footer": { "text": fit(&footer, MAX_FOOTER, Unit::Utf16, Escape::Plain) },
    });
    if !message.title.trim().is_empty() {
        embed["title"] = json!(fit(&message.title, MAX_TITLE, Unit::Utf16, Escape::Plain));
    }
    let body = message.body.trim_end();
    if !body.is_empty() {
        embed["description"] = json!(fit(body, MAX_DESCRIPTION, Unit::Utf16, Escape::Plain));
    }
    if let Some(link) = message.web_link() {
        embed["url"] = json!(link);
    }
    json!({ "embeds": [embed] })
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{CaptureServer, deliver, message};
    use super::super::{ChannelConfig, Severity};
    use super::*;

    fn discord(server: &CaptureServer) -> ChannelConfig {
        ChannelConfig::Discord {
            webhook_url: server.url("/api/webhooks/123/discord-token-abcdef"),
        }
    }

    #[test]
    fn embed_has_color_timestamp_link_and_footer() {
        assert_eq!(
            payload(&message()),
            json!({
                "embeds": [{
                    "title": "Plan <ready> & waiting",
                    "description": "买入 AAPL 10 股 @ 180.00\nSell MSFT if P/E > 35 & RSI < 30",
                    "color": 0xA37B12,
                    "timestamp": "2026-10-05T13:30:00Z",
                    "url": "https://quant.example.com/plans/42?slot=open&mode=review",
                    "footer": { "text": "hone-quant · rebalance" },
                }]
            })
        );
    }

    #[test]
    fn colors_follow_severity() {
        let cases = [
            (Severity::Info, 0x5B8DEF),
            (Severity::Success, 0x1F7A45),
            (Severity::Warning, 0xA37B12),
            (Severity::Critical, 0xB94432),
        ];
        for (severity, color) in cases {
            let message = OutboundMessage {
                severity,
                ..message()
            };
            assert_eq!(payload(&message)["embeds"][0]["color"], color);
        }
    }

    #[test]
    fn description_is_cut_to_4096_characters() {
        let message = OutboundMessage {
            body: "纳斯达克100指数成分股调整".repeat(1000),
            link: None,
            ..message()
        };
        let embed = &payload(&message)["embeds"][0];
        let description = embed["description"].as_str().unwrap();
        assert!(description.chars().count() <= MAX_DESCRIPTION);
        assert!(description.ends_with('…'));
        assert!(embed.get("url").is_none());
    }

    #[tokio::test]
    async fn no_content_reply_is_a_success() {
        let server = CaptureServer::start(204, "text/plain", "").await;
        deliver(&discord(&server), &message(), &server)
            .await
            .unwrap();

        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/webhooks/123/discord-token-abcdef");
        assert_eq!(request.json(), payload(&message()));
    }

    #[tokio::test]
    async fn error_reports_discords_message() {
        let server =
            CaptureServer::json(404, r#"{"message": "Unknown Webhook", "code": 10015}"#).await;
        let err = deliver(&discord(&server), &message(), &server)
            .await
            .unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert_eq!(
            err.to_string(),
            "delivery failed: Discord rejected the message (HTTP 404): Unknown Webhook"
        );
    }
}
