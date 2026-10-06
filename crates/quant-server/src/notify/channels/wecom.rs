//! WeCom (企业微信) group robot, sent as a markdown message.

use serde_json::json;

use super::format::{Escape, Unit, fit};
use super::{ChannelError, OutboundMessage, transport};

/// WeCom's limit on `markdown.content`, in UTF-8 bytes.
const MAX_CONTENT_BYTES: usize = 4096;
/// Titles are one line in practice; this keeps a runaway one from crowding out the body.
const MAX_TITLE_BYTES: usize = 512;
/// Opens the quoted body after the title line.
const QUOTE_START: &str = "\n> ";

pub(super) async fn send(
    http: &reqwest::Client,
    webhook_url: &str,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let payload = json!({ "msgtype": "markdown", "markdown": { "content": content(message) } });
    let reply = transport::send(http.post(webhook_url).json(&payload)).await?;
    let body = reply.json();
    let code = body["errcode"].as_i64();
    if code == Some(0) && reply.status.is_success() {
        return Ok(());
    }
    let detail = code.map(|code| {
        let errmsg = body["errmsg"].as_str().unwrap_or("no message");
        format!("errcode {code}: {errmsg}")
    });
    Err(reply.rejected("WeCom", detail))
}

/// `**title**`, the body as a block quote, then a link — cut to fit [`MAX_CONTENT_BYTES`].
fn content(message: &OutboundMessage) -> String {
    let title = fit(&message.title, MAX_TITLE_BYTES, Unit::Bytes, Escape::Plain);
    let mut content = format!("**{title}**");
    let link = message
        .web_link()
        .map(|link| format!("\n[打开 hone-quant]({link})"))
        .unwrap_or_default();
    let used = content.len() + QUOTE_START.len() + link.len();
    let body = fit(
        message.body.trim_end(),
        MAX_CONTENT_BYTES.saturating_sub(used),
        Unit::Bytes,
        Escape::Quote,
    );
    if !body.is_empty() {
        content.push_str(QUOTE_START);
        content.push_str(&body);
    }
    content.push_str(&link);
    content
}

#[cfg(test)]
mod tests {
    use super::super::ChannelConfig;
    use super::super::testkit::{CaptureServer, deliver, message};
    use super::*;

    fn wecom(server: &CaptureServer) -> ChannelConfig {
        ChannelConfig::Wecom {
            webhook_url: server.url("/cgi-bin/webhook/send?key=wecom-key-0123456789"),
        }
    }

    #[test]
    fn content_is_bold_title_quoted_body_and_link() {
        assert_eq!(
            content(&message()),
            "**Plan <ready> & waiting**\n\
             > 买入 AAPL 10 股 @ 180.00\n\
             > Sell MSFT if P/E > 35 & RSI < 30\n\
             [打开 hone-quant](https://quant.example.com/plans/42?slot=open&mode=review)"
        );
    }

    #[test]
    fn title_alone_when_there_is_no_body_or_link() {
        let message = OutboundMessage {
            body: String::new(),
            link: None,
            ..message()
        };
        assert_eq!(content(&message), "**Plan <ready> & waiting**");
    }

    #[test]
    fn content_fits_4096_bytes_of_chinese() {
        let message = OutboundMessage {
            body: "风险提示：组合回撤超过阈值，请检查持仓\n".repeat(300),
            ..message()
        };
        let content = content(&message);
        assert!(content.len() <= MAX_CONTENT_BYTES, "{}", content.len());
        assert!(content.contains("…\n[打开 hone-quant]("));
        assert!(
            content
                .lines()
                .skip(1)
                .all(|line| line.starts_with("> ") || line.starts_with('['))
        );
    }

    #[tokio::test]
    async fn posts_markdown_and_accepts_errcode_zero() {
        let server = CaptureServer::json(200, r#"{"errcode":0,"errmsg":"ok"}"#).await;
        deliver(&wecom(&server), &message(), &server).await.unwrap();

        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.target,
            "/cgi-bin/webhook/send?key=wecom-key-0123456789"
        );
        assert_eq!(
            request.json(),
            json!({ "msgtype": "markdown", "markdown": { "content": content(&message()) } })
        );
    }

    #[tokio::test]
    async fn nonzero_errcode_is_an_error_without_the_key() {
        let reply = r#"{"errcode":93000,"errmsg":"invalid webhook url, key wecom-key-0123456789"}"#;
        let server = CaptureServer::json(200, reply).await;
        let err = deliver(&wecom(&server), &message(), &server)
            .await
            .unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert_eq!(
            err.to_string(),
            "delivery failed: WeCom rejected the message (HTTP 200): \
             errcode 93000: invalid webhook url, key ••••"
        );
    }
}
