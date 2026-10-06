//! Telegram Bot API `sendMessage`, formatted as HTML.
//!
//! HTML rather than MarkdownV2: HTML reserves three characters, while MarkdownV2 reserves
//! eighteen that are everywhere in trading text (`.`, `-`, `+`, `(`, …) and fails the whole
//! message when one is left unescaped.

use serde_json::{Value, json};

use super::format::{Escape, Unit, fit};
use super::{ChannelError, OutboundMessage, transport};

/// Telegram's limit on a message's text.
const MAX_TEXT: usize = 4096;
/// Titles are one line in practice; this keeps a runaway one from crowding out the body.
const MAX_TITLE: usize = 256;

pub(super) async fn send(
    http: &reqwest::Client,
    api_base: &str,
    token: &str,
    chat_id: &str,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let url = format!("{}/bot{token}/sendMessage", api_base.trim_end_matches('/'));
    let reply = transport::send(http.post(url).json(&payload(chat_id, message))).await?;
    let body = reply.json();
    if body["ok"] == true {
        return Ok(());
    }
    let description = body["description"].as_str().map(str::to_string);
    Err(reply.rejected("Telegram", description))
}

fn payload(chat_id: &str, message: &OutboundMessage) -> Value {
    json!({
        "chat_id": chat_id,
        "text": text(message),
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
    })
}

/// `{emoji} <b>{title}</b>`, the body, then a link — escaped, and cut to fit [`MAX_TEXT`].
fn text(message: &OutboundMessage) -> String {
    let title = fit(&message.title, MAX_TITLE, Unit::Utf16, Escape::Html);
    let head = format!("{} <b>{title}</b>", message.severity.emoji());
    let link = message
        .web_link()
        .map(|link| {
            let href = Escape::HtmlAttr.apply(link);
            format!("\n<a href=\"{href}\">Open hone-quant</a>")
        })
        .unwrap_or_default();
    // Telegram applies the limit after stripping tags, so counting them leaves a safe margin.
    let used = Unit::Utf16.measure(&head) + Unit::Utf16.measure(&link) + 1;
    let body = fit(
        message.body.trim_end(),
        MAX_TEXT.saturating_sub(used),
        Unit::Utf16,
        Escape::Html,
    );
    if body.is_empty() {
        format!("{head}{link}")
    } else {
        format!("{head}\n{body}{link}")
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::{CaptureServer, deliver, message};
    use super::super::{ChannelConfig, Severity};
    use super::*;

    fn telegram() -> ChannelConfig {
        ChannelConfig::Telegram {
            bot_token: "123456:ABC-secret_token".into(),
            chat_id: " -100200300 ".into(),
        }
    }

    #[test]
    fn text_is_escaped_html_with_emoji_title_body_and_link() {
        assert_eq!(
            text(&message()),
            "⚠️ <b>Plan &lt;ready&gt; &amp; waiting</b>\n\
             买入 AAPL 10 股 @ 180.00\n\
             Sell MSFT if P/E &gt; 35 &amp; RSI &lt; 30\n\
             <a href=\"https://quant.example.com/plans/42?slot=open&amp;mode=review\">\
             Open hone-quant</a>"
        );
    }

    #[test]
    fn every_severity_has_its_emoji() {
        let cases = [
            (Severity::Info, "ℹ️"),
            (Severity::Success, "✅"),
            (Severity::Warning, "⚠️"),
            (Severity::Critical, "🚨"),
        ];
        for (severity, emoji) in cases {
            let message = OutboundMessage {
                severity,
                ..message()
            };
            assert!(text(&message).starts_with(&format!("{emoji} <b>")));
        }
    }

    #[test]
    fn empty_body_and_missing_link_are_omitted() {
        let message = OutboundMessage {
            body: "\n".into(),
            link: None,
            ..message()
        };
        assert_eq!(text(&message), "⚠️ <b>Plan &lt;ready&gt; &amp; waiting</b>");
    }

    #[test]
    fn long_body_is_cut_to_the_limit_and_keeps_the_link() {
        let message = OutboundMessage {
            body: "组合回撤 > 5% & 风险提示。".repeat(1000),
            ..message()
        };
        let text = text(&message);
        assert!(text.encode_utf16().count() <= MAX_TEXT, "{}", text.len());
        assert!(text.contains("…\n<a href="));
        assert!(text.ends_with("Open hone-quant</a>"));
        for (at, _) in text.match_indices('&') {
            let rest = &text[at..];
            assert!(
                ["&amp;", "&lt;", "&gt;"]
                    .iter()
                    .any(|entity| rest.starts_with(entity)),
                "broken entity at {at}"
            );
        }
    }

    #[tokio::test]
    async fn posts_html_message_to_the_bot_endpoint() {
        let server = CaptureServer::json(200, r#"{"ok":true,"result":{"message_id":7}}"#).await;
        deliver(&telegram(), &message(), &server).await.unwrap();

        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/bot123456:ABC-secret_token/sendMessage");
        assert_eq!(request.header("content-type"), Some("application/json"));
        let body = request.json();
        assert_eq!(body["chat_id"], "-100200300");
        assert_eq!(body["parse_mode"], "HTML");
        assert_eq!(body["disable_web_page_preview"], true);
        assert_eq!(body["text"], text(&message()));
    }

    #[tokio::test]
    async fn ok_false_is_an_error_with_the_description() {
        let reply = r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#;
        let server = CaptureServer::json(400, reply).await;
        let err = deliver(&telegram(), &message(), &server).await.unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert_eq!(
            err.to_string(),
            "delivery failed: Telegram rejected the message (HTTP 400): Bad Request: chat not found"
        );
    }

    #[tokio::test]
    async fn errors_never_repeat_the_token() {
        let reply = r#"{"ok":false,"error_code":401,
                        "description":"Unauthorized: bot123456:ABC-secret_token was revoked"}"#;
        let server = CaptureServer::json(401, reply).await;
        let err = deliver(&telegram(), &message(), &server)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unauthorized: bot•••• was revoked"), "{err}");
        assert!(!err.contains("ABC-secret_token"), "{err}");
    }

    #[tokio::test]
    async fn a_reply_that_is_not_json_is_an_error() {
        let server = CaptureServer::text(502, "<html>\n  Bad Gateway\n</html>").await;
        let err = deliver(&telegram(), &message(), &server)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.ends_with("(HTTP 502): <html> Bad Gateway </html>"),
            "{err}"
        );
    }
}
