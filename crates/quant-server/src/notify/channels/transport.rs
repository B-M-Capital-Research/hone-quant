//! The one HTTP exchange behind every channel but email: send with a timeout, read a bounded
//! reply, and describe failures without the request URL.

use std::error::Error as _;

use reqwest::{RequestBuilder, StatusCode};
use serde_json::Value;

use super::format::{Escape, Unit, fit};
use super::{ChannelError, REQUEST_TIMEOUT};

/// Replies are read up to this size: plenty for any API error, and a misbehaving endpoint cannot
/// make the server buffer an unbounded body.
const MAX_REPLY_BYTES: usize = 64 * 1024;
/// Longest excerpt of a reply quoted in an error.
const MAX_EXCERPT: usize = 300;

/// A platform's answer.
pub(super) struct Reply {
    pub status: StatusCode,
    /// Body text, lossily decoded and capped at [`MAX_REPLY_BYTES`].
    pub body: String,
}

impl Reply {
    /// The body as JSON; `Value::Null` when it is not JSON, so fields read as absent.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// The error for a reply that is not a success: `detail` when the platform explained itself,
    /// else an excerpt of the body.
    pub fn rejected(&self, service: &str, detail: Option<String>) -> ChannelError {
        let detail = detail.unwrap_or_else(|| self.excerpt());
        ChannelError::Delivery(format!(
            "{service} rejected the message (HTTP {}): {detail}",
            self.status.as_u16()
        ))
    }

    fn excerpt(&self) -> String {
        let line = self.body.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            return "empty reply".to_string();
        }
        fit(&line, MAX_EXCERPT, Unit::Utf16, Escape::Plain)
    }
}

/// Sends `request` with [`REQUEST_TIMEOUT`] and reads the reply, whatever its status.
///
/// A redirect is a failure: reqwest follows it, but re-sends a POST answered with 301, 302 or 303
/// as a GET without a body, so a success at the new address does not mean the message arrived.
pub(super) async fn send(request: RequestBuilder) -> Result<Reply, ChannelError> {
    let (client, request) = request.timeout(REQUEST_TIMEOUT).build_split();
    let request = request.map_err(request_error)?;
    let target = request.url().clone();
    let mut response = client.execute(request).await.map_err(request_error)?;
    if *response.url() != target {
        return Err(ChannelError::Delivery(
            "the endpoint redirected the request; configure its final URL".to_string(),
        ));
    }
    let status = response.status();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        let room = MAX_REPLY_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if body.len() == MAX_REPLY_BYTES {
            break;
        }
    }
    Ok(Reply {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Describes a transport failure with its causes but without the URL, which may be a credential.
fn request_error(err: reqwest::Error) -> ChannelError {
    if err.is_timeout() {
        return ChannelError::Delivery(format!("no reply within {}s", REQUEST_TIMEOUT.as_secs()));
    }
    let err = err.without_url();
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    ChannelError::Delivery(text)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::response::Redirect;
    use axum::routing::{get, post};

    use super::super::testkit::{CaptureServer, client};
    use super::*;

    #[tokio::test]
    async fn a_redirect_is_a_failure_even_when_the_target_succeeds() {
        // 303 makes reqwest re-send the POST as a bodiless GET, which this target accepts.
        let app = Router::new()
            .route("/hook", post(|| async { Redirect::to("/moved") }))
            .route("/moved", get(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let request = client()
            .post(url)
            .json(&serde_json::json!({ "text": "hi" }));
        let err = send(request).await.err().expect("a redirect must fail");
        assert_eq!(
            err.to_string(),
            "delivery failed: the endpoint redirected the request; configure its final URL"
        );
    }

    #[tokio::test]
    async fn replies_are_read_up_to_the_cap() {
        let huge: &'static str = Box::leak("x".repeat(3 * MAX_REPLY_BYTES).into_boxed_str());
        let server = CaptureServer::text(200, huge).await;
        let reply = send(client().post(server.url("/"))).await.unwrap();
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body.len(), MAX_REPLY_BYTES);
        let excerpt = reply.excerpt();
        assert!(excerpt.chars().count() <= MAX_EXCERPT && excerpt.ends_with('…'));
    }
}
