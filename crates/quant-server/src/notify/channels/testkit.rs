//! Test support: a loopback HTTP server that records every request and answers with a canned
//! reply, plus a sample message and one sample configuration per channel kind.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use chrono::{TimeZone, Utc};
use serde_json::Value;

use super::{ChannelConfig, ChannelError, EmailTls, Endpoints, OutboundMessage, Severity};

/// One request as the capture server saw it.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: Method,
    /// Path and query.
    pub target: String,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Recorded {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

pub struct CaptureServer {
    base: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl CaptureServer {
    /// Starts a server on `127.0.0.1:0` answering every request with `status` and `body`.
    pub async fn start(status: u16, content_type: &'static str, body: &'static str) -> Self {
        let status = StatusCode::from_u16(status).expect("valid status");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        let app = Router::new().fallback(
            move |method: Method, uri: Uri, headers: HeaderMap, request_body: Bytes| {
                let log = Arc::clone(&log);
                async move {
                    log.lock().unwrap().push(Recorded {
                        method,
                        target: uri.to_string(),
                        headers,
                        body: request_body,
                    });
                    (status, [(header::CONTENT_TYPE, content_type)], body)
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let base = format!("http://{}", listener.local_addr().expect("local address"));
        tokio::spawn(async move { axum::serve(listener, app).await.expect("capture server") });
        Self { base, requests }
    }

    pub async fn json(status: u16, body: &'static str) -> Self {
        Self::start(status, "application/json", body).await
    }

    pub async fn text(status: u16, body: &'static str) -> Self {
        Self::start(status, "text/plain; charset=utf-8", body).await
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The one request received so far.
    pub fn request(&self) -> Recorded {
        let requests = self.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "expected exactly one request");
        requests[0].clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

/// A client that ignores the sandbox's `HTTPS_PROXY`, so loopback requests stay local.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("reqwest client")
}

/// Sends `message` through `config`, with Telegram's API pointed at `server`.
pub async fn deliver(
    config: &ChannelConfig,
    message: &OutboundMessage,
    server: &CaptureServer,
) -> Result<(), ChannelError> {
    let endpoints = Endpoints {
        telegram_api_base: server.base.clone(),
    };
    super::send_with(&client(), config, message, &endpoints).await
}

/// Markup characters, mixed Chinese and English, and a link with a query string.
pub fn message() -> OutboundMessage {
    OutboundMessage {
        title: "Plan <ready> & waiting".into(),
        body: "买入 AAPL 10 股 @ 180.00\nSell MSFT if P/E > 35 & RSI < 30".into(),
        severity: Severity::Warning,
        category: "rebalance".into(),
        link: Some("https://quant.example.com/plans/42?slot=open&mode=review".into()),
        timestamp: Utc.with_ymd_and_hms(2026, 10, 5, 13, 30, 0).unwrap(),
    }
}

/// One valid configuration per kind, in declaration order.
pub fn samples() -> Vec<ChannelConfig> {
    vec![
        ChannelConfig::Telegram {
            bot_token: "123456:ABC-telegram-token".into(),
            chat_id: "-100200300".into(),
        },
        ChannelConfig::Feishu {
            webhook_url: "https://open.feishu.cn/open-apis/bot/v2/hook/feishu-hook-token".into(),
            secret: Some("feishu-signing-secret".into()),
        },
        ChannelConfig::Wecom {
            webhook_url: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=wecom-key-0123"
                .into(),
        },
        ChannelConfig::Slack {
            webhook_url: "https://hooks.slack.com/services/T0001/B0002/slack-secret-xyz9".into(),
        },
        ChannelConfig::Discord {
            webhook_url: "https://discord.com/api/webhooks/123/discord-token-abcd".into(),
        },
        ChannelConfig::Webhook {
            url: "https://hooks.example.com/in/receiver-token-5678".into(),
            secret: Some("whsec-signing".into()),
        },
        ChannelConfig::Email {
            host: "smtp.example.com".into(),
            port: 587,
            username: Some("alerts@example.com".into()),
            password: Some("smtp-password-1234".into()),
            from: "hone-quant <alerts@example.com>".into(),
            to: vec!["ops@example.com".into()],
            tls: EmailTls::StartTls,
        },
    ]
}
