//! Feishu / Lark custom bot, sent as a rich-text `post`, optionally signed.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use super::format::{Escape, Unit, fit};
use super::{ChannelError, OutboundMessage, transport};

/// Feishu rejects request bodies over 20 KB; capping the body well below that leaves room for
/// the JSON around it.
const MAX_BODY_BYTES: usize = 12_000;
const LINK_TEXT: &str = "打开 hone-quant / Open";

pub(super) async fn send(
    http: &reqwest::Client,
    webhook_url: &str,
    secret: Option<&str>,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let signature = secret.map(|secret| {
        let timestamp = Utc::now().timestamp();
        (timestamp, sign(timestamp, secret))
    });
    let reply = transport::send(http.post(webhook_url).json(&payload(message, signature))).await?;
    let body = reply.json();
    // Current deployments answer `code`/`msg`; older ones `StatusCode`/`StatusMessage`.
    let code = body
        .get("code")
        .or_else(|| body.get("StatusCode"))
        .and_then(Value::as_i64);
    if code == Some(0) && reply.status.is_success() {
        return Ok(());
    }
    let detail = code.map(|code| {
        let msg = body
            .get("msg")
            .or_else(|| body.get("StatusMessage"))
            .and_then(Value::as_str)
            .unwrap_or("no message");
        format!("code {code}: {msg}")
    });
    Err(reply.rejected("Feishu", detail))
}

/// One paragraph per body line, then the link; `signature` is `(timestamp, sign)`.
fn payload(message: &OutboundMessage, signature: Option<(i64, String)>) -> Value {
    let body = fit(
        message.body.trim_end(),
        MAX_BODY_BYTES,
        Unit::Bytes,
        Escape::Plain,
    );
    let mut paragraphs: Vec<Value> = body
        .lines()
        .map(|line| json!([{ "tag": "text", "text": line }]))
        .collect();
    if let Some(link) = message.web_link() {
        paragraphs.push(json!([{ "tag": "a", "text": LINK_TEXT, "href": link }]));
    }
    let mut payload = json!({
        "msg_type": "post",
        "content": { "post": { "zh_cn": { "title": message.title, "content": paragraphs } } },
    });
    if let Some((timestamp, sign)) = signature {
        payload["timestamp"] = json!(timestamp.to_string());
        payload["sign"] = json!(sign);
    }
    payload
}

/// Feishu's documented signature: HMAC-SHA256 keyed with `"{timestamp}\n{secret}"` over an
/// *empty* message, base64-encoded. The secret is part of the key, not the message, and Feishu
/// rejects timestamps more than an hour from its own clock.
fn sign(timestamp: i64, secret: &str) -> String {
    let key = format!("{timestamp}\n{secret}");
    let mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC takes any key length");
    BASE64.encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::super::ChannelConfig;
    use super::super::testkit::{CaptureServer, deliver, message};
    use super::*;

    fn feishu(server: &CaptureServer, secret: Option<&str>) -> ChannelConfig {
        ChannelConfig::Feishu {
            webhook_url: server.url("/open-apis/bot/v2/hook/feishu-hook-token"),
            secret: secret.map(str::to_string),
        }
    }

    #[test]
    fn sign_matches_an_independent_reference() {
        // Computed with Python's hmac module and with `openssl dgst -sha256 -mac HMAC`.
        assert_eq!(
            sign(1599360473, "lark-secret"),
            "D/eEn4Cv6I563yQzE4oPoWsfVj8xiAWEikXmDMg0N34="
        );
    }

    #[test]
    fn payload_has_one_paragraph_per_line_then_the_link() {
        let payload = payload(&message(), None);
        assert_eq!(payload["msg_type"], "post");
        let post = &payload["content"]["post"]["zh_cn"];
        assert_eq!(post["title"], "Plan <ready> & waiting");
        assert_eq!(
            post["content"],
            json!([
                [{ "tag": "text", "text": "买入 AAPL 10 股 @ 180.00" }],
                [{ "tag": "text", "text": "Sell MSFT if P/E > 35 & RSI < 30" }],
                [{
                    "tag": "a",
                    "text": "打开 hone-quant / Open",
                    "href": "https://quant.example.com/plans/42?slot=open&mode=review"
                }],
            ])
        );
        assert!(payload.get("timestamp").is_none() && payload.get("sign").is_none());
    }

    #[test]
    fn long_body_is_capped_in_bytes() {
        let message = OutboundMessage {
            body: "美股盘前异动提醒".repeat(1000),
            link: None,
            ..message()
        };
        let payload = payload(&message, None);
        let text = payload["content"]["post"]["zh_cn"]["content"][0][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.len() <= MAX_BODY_BYTES && text.ends_with('…'));
    }

    #[tokio::test]
    async fn signed_post_carries_a_valid_timestamp_and_sign() {
        let server = CaptureServer::json(200, r#"{"code":0,"msg":"success","data":{}}"#).await;
        deliver(&feishu(&server, Some("lark-secret")), &message(), &server)
            .await
            .unwrap();

        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/open-apis/bot/v2/hook/feishu-hook-token");
        let body = request.json();
        assert_eq!(body["msg_type"], "post");
        assert_eq!(body["content"], payload(&message(), None)["content"]);
        let timestamp = body["timestamp"].as_str().expect("timestamp is a string");
        let seconds: i64 = timestamp.parse().unwrap();
        assert!((Utc::now().timestamp() - seconds).abs() < 60);
        // Recompute the signature from the documented recipe, not with `sign`.
        let mut mac =
            Hmac::<Sha256>::new_from_slice(format!("{timestamp}\nlark-secret").as_bytes()).unwrap();
        mac.update(b"");
        assert_eq!(body["sign"], BASE64.encode(mac.finalize().into_bytes()));
    }

    #[tokio::test]
    async fn legacy_status_code_reply_is_a_success() {
        let reply = r#"{"Extra":null,"StatusCode":0,"StatusMessage":"success"}"#;
        let server = CaptureServer::json(200, reply).await;
        deliver(&feishu(&server, None), &message(), &server)
            .await
            .unwrap();
        assert!(server.request().json().get("sign").is_none());
    }

    #[tokio::test]
    async fn nonzero_code_is_an_error_with_the_message() {
        let reply = r#"{"code":19021,"data":{},
            "msg":"sign match fail or timestamp is not within one hour from current time"}"#;
        let server = CaptureServer::json(200, reply).await;
        let err = deliver(&feishu(&server, Some("stale-secret")), &message(), &server)
            .await
            .unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert_eq!(
            err.to_string(),
            "delivery failed: Feishu rejected the message (HTTP 200): code 19021: \
             sign match fail or timestamp is not within one hour from current time"
        );
    }

    #[tokio::test]
    async fn rejection_without_a_code_quotes_the_reply() {
        let server = CaptureServer::text(404, "404 page not found").await;
        let err = deliver(&feishu(&server, None), &message(), &server)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.ends_with("(HTTP 404): 404 page not found"), "{err}");
    }
}
