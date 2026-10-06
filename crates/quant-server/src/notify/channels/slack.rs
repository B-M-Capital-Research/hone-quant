//! Slack incoming webhook, laid out with Block Kit, with a plain fallback for notifications.

use serde_json::{Value, json};

use super::format::{Escape, Unit, fit};
use super::{ChannelError, OutboundMessage, transport};

/// Block Kit limits: header text, and section or fallback text.
const MAX_HEADER: usize = 150;
const MAX_TEXT: usize = 3000;
/// Categories are short labels; this keeps the context line to one line.
const MAX_CATEGORY: usize = 100;

pub(super) async fn send(
    http: &reqwest::Client,
    webhook_url: &str,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let reply = transport::send(http.post(webhook_url).json(&payload(message))).await?;
    // Slack answers `200 ok`; failures are 4xx/5xx with a short code such as `no_service`.
    if reply.status.is_success() {
        Ok(())
    } else {
        Err(reply.rejected("Slack", None))
    }
}

/// Header, body, a context line with severity and category, and a link button. Header and
/// section are skipped when empty: Slack rejects the whole message over an empty text field.
fn payload(message: &OutboundMessage) -> Value {
    let emoji = message.severity.emoji();
    let body = message.body.trim_end();
    let mut blocks = Vec::new();
    if !message.title.trim().is_empty() {
        let title = fit(&message.title, MAX_HEADER, Unit::Utf16, Escape::Plain);
        blocks.push(json!({
            "type": "header",
            "text": { "type": "plain_text", "text": title, "emoji": true },
        }));
    }
    if !body.is_empty() {
        let text = fit(body, MAX_TEXT, Unit::Utf16, Escape::Html);
        blocks.push(json!({ "type": "section", "text": { "type": "mrkdwn", "text": text } }));
    }
    let category = fit(&message.category, MAX_CATEGORY, Unit::Utf16, Escape::Html);
    let context = format!("{emoji} *{}* · {category}", message.severity.as_str());
    blocks.push(json!({ "type": "context", "elements": [{ "type": "mrkdwn", "text": context }] }));
    if let Some(link) = message.web_link() {
        blocks.push(json!({
            "type": "actions",
            "elements": [{
                "type": "button",
                "text": { "type": "plain_text", "text": "Open hone-quant" },
                "url": link,
            }],
        }));
    }
    let fallback = format!("{emoji} {}\n{body}", message.title);
    json!({
        "text": fit(fallback.trim_end(), MAX_TEXT, Unit::Utf16, Escape::Html),
        "blocks": blocks,
    })
}

#[cfg(test)]
mod tests {
    use super::super::ChannelConfig;
    use super::super::testkit::{CaptureServer, deliver, message};
    use super::*;

    fn slack(server: &CaptureServer) -> ChannelConfig {
        ChannelConfig::Slack {
            webhook_url: server.url("/services/T0001/B0002/slack-secret-xyz9"),
        }
    }

    #[test]
    fn payload_has_header_section_context_and_button() {
        let payload = payload(&message());
        assert_eq!(
            payload["text"],
            "⚠️ Plan &lt;ready&gt; &amp; waiting\n买入 AAPL 10 股 @ 180.00\n\
             Sell MSFT if P/E &gt; 35 &amp; RSI &lt; 30"
        );
        assert_eq!(
            payload["blocks"],
            json!([
                {
                    "type": "header",
                    "text": {
                        "type": "plain_text",
                        "text": "Plan <ready> & waiting",
                        "emoji": true,
                    },
                },
                {
                    "type": "section",
                    "text": {
                        "type": "mrkdwn",
                        "text": "买入 AAPL 10 股 @ 180.00\nSell MSFT if P/E &gt; 35 &amp; RSI &lt; 30",
                    },
                },
                {
                    "type": "context",
                    "elements": [{ "type": "mrkdwn", "text": "⚠️ *warning* · rebalance" }],
                },
                {
                    "type": "actions",
                    "elements": [{
                        "type": "button",
                        "text": { "type": "plain_text", "text": "Open hone-quant" },
                        "url": "https://quant.example.com/plans/42?slot=open&mode=review",
                    }],
                },
            ])
        );
    }

    #[test]
    fn empty_parts_are_left_out() {
        let message = OutboundMessage {
            title: " ".into(),
            body: String::new(),
            link: None,
            ..message()
        };
        let payload = payload(&message);
        let blocks = payload["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "context");
    }

    #[test]
    fn long_texts_respect_block_limits() {
        let message = OutboundMessage {
            title: "季度再平衡".repeat(100),
            body: "持仓明细 & 成交回报\n".repeat(500),
            ..message()
        };
        let payload = payload(&message);
        let header = payload["blocks"][0]["text"]["text"].as_str().unwrap();
        let section = payload["blocks"][1]["text"]["text"].as_str().unwrap();
        assert!(header.encode_utf16().count() <= MAX_HEADER && header.ends_with('…'));
        assert!(section.encode_utf16().count() <= MAX_TEXT && section.ends_with('…'));
        assert!(payload["text"].as_str().unwrap().encode_utf16().count() <= MAX_TEXT);
    }

    #[tokio::test]
    async fn posts_blocks_and_accepts_ok() {
        let server = CaptureServer::text(200, "ok").await;
        deliver(&slack(&server), &message(), &server).await.unwrap();

        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/services/T0001/B0002/slack-secret-xyz9");
        assert_eq!(request.json(), payload(&message()));
    }

    #[tokio::test]
    async fn error_status_is_reported_with_slacks_code() {
        let server = CaptureServer::text(404, "no_service").await;
        let err = deliver(&slack(&server), &message(), &server)
            .await
            .unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert_eq!(
            err.to_string(),
            "delivery failed: Slack rejected the message (HTTP 404): no_service"
        );
    }
}
