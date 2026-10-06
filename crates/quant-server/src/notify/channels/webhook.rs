//! Generic JSON webhook, for anything without a dedicated channel (n8n, Home Assistant, a custom
//! bot, …).
//!
//! With a secret configured, each request is signed so the receiver can check origin and
//! freshness: `X-Hone-Quant-Signature: sha256=<hex HMAC-SHA256(secret, "{timestamp}.{body}")>`,
//! with the Unix timestamp in `X-Hone-Quant-Timestamp`. Covering the timestamp lets receivers
//! reject replays; signing the exact bytes sent means receivers must verify the raw body before
//! parsing it.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use sha2::Sha256;

use super::{ChannelError, OutboundMessage, Severity, transport};

const TIMESTAMP_HEADER: &str = "X-Hone-Quant-Timestamp";
const SIGNATURE_HEADER: &str = "X-Hone-Quant-Signature";

#[derive(Serialize)]
struct Payload<'a> {
    source: &'static str,
    title: &'a str,
    body: &'a str,
    severity: Severity,
    category: &'a str,
    link: Option<&'a str>,
    timestamp: DateTime<Utc>,
}

pub(super) async fn send(
    http: &reqwest::Client,
    url: &str,
    secret: Option<&str>,
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let payload = Payload {
        source: "hone-quant",
        title: &message.title,
        body: &message.body,
        severity: message.severity,
        category: &message.category,
        link: message.web_link(),
        timestamp: message.timestamp,
    };
    let body = serde_json::to_vec(&payload)
        .map_err(|err| ChannelError::Delivery(format!("cannot encode the payload: {err}")))?;
    let mut request = http.post(url).header(CONTENT_TYPE, "application/json");
    if let Some(secret) = secret {
        let timestamp = Utc::now().timestamp();
        request = request
            .header(TIMESTAMP_HEADER, timestamp.to_string())
            .header(SIGNATURE_HEADER, signature(secret, timestamp, &body));
    }
    let reply = transport::send(request.body(body)).await?;
    if reply.status.is_success() {
        Ok(())
    } else {
        Err(reply.rejected("Webhook", None))
    }
}

fn signature(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::ChannelConfig;
    use super::super::testkit::{CaptureServer, deliver, message};
    use super::*;

    fn webhook(server: &CaptureServer, secret: Option<&str>) -> ChannelConfig {
        ChannelConfig::Webhook {
            url: server.url("/hooks/receiver-token-123"),
            secret: secret.map(str::to_string),
        }
    }

    #[test]
    fn signature_matches_an_independent_reference() {
        // Computed with Python's hmac module and with `openssl dgst -sha256 -hmac`.
        assert_eq!(
            signature("whsec-test", 1_700_000_000, br#"{"a":1}"#),
            "sha256=fed7a1a6c799a6367d73724d76bc70c16678908a32a879cd1ded6565ac05cece"
        );
    }

    #[tokio::test]
    async fn posts_signed_json() {
        let server = CaptureServer::json(202, "{}").await;
        deliver(&webhook(&server, Some("whsec-test")), &message(), &server)
            .await
            .unwrap();

        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/hooks/receiver-token-123");
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(
            request.json(),
            json!({
                "source": "hone-quant",
                "title": "Plan <ready> & waiting",
                "body": "买入 AAPL 10 股 @ 180.00\nSell MSFT if P/E > 35 & RSI < 30",
                "severity": "warning",
                "category": "rebalance",
                "link": "https://quant.example.com/plans/42?slot=open&mode=review",
                "timestamp": "2026-10-05T13:30:00Z",
            })
        );
        // Verify the way a receiver would: HMAC over "{timestamp}." and the raw body.
        let timestamp = request
            .header("x-hone-quant-timestamp")
            .expect("timestamp header");
        let seconds: i64 = timestamp.parse().unwrap();
        assert!((Utc::now().timestamp() - seconds).abs() < 60);
        let mut mac = Hmac::<Sha256>::new_from_slice(b"whsec-test").unwrap();
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(&request.body);
        let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert_eq!(
            request.header("x-hone-quant-signature"),
            Some(expected.as_str())
        );
    }

    #[tokio::test]
    async fn unsigned_without_a_secret() {
        for secret in [None, Some(""), Some("  ")] {
            let server = CaptureServer::json(200, "{}").await;
            deliver(&webhook(&server, secret), &message(), &server)
                .await
                .unwrap();
            let request = server.request();
            assert!(request.header("x-hone-quant-signature").is_none());
            assert!(request.header("x-hone-quant-timestamp").is_none());
        }
    }

    #[tokio::test]
    async fn server_error_is_reported_without_the_url() {
        let server = CaptureServer::text(500, "Cannot POST /hooks/receiver-token-123").await;
        let err = deliver(&webhook(&server, None), &message(), &server)
            .await
            .unwrap_err();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert_eq!(
            err.to_string(),
            "delivery failed: Webhook rejected the message (HTTP 500): Cannot POST ••••"
        );
    }
}
