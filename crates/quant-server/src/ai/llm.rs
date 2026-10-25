//! A minimal client for an OpenAI-compatible `/chat/completions` endpoint.
//!
//! In production this is honeclaw's Gemini route: the loopback proxy that turns OpenAI-shaped
//! requests into Gemini calls (`response_format: json_object` becomes Gemini's JSON mode,
//! `reasoning_effort` its thinking level). Only unary, JSON-mode requests are needed here.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::config::LlmConfig;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("model request failed: {0}")]
    Transport(String),
    #[error("model endpoint answered HTTP {status}: {detail}")]
    Status { status: u16, detail: String },
    #[error("unexpected model response: {0}")]
    Parse(String),
}

/// One answer, with what it cost.
#[derive(Debug, Clone)]
pub struct Completion {
    pub content: String,
    /// The model the endpoint reports (may differ from the one requested).
    pub model: String,
    pub usage: Option<Value>,
    pub finish_reason: Option<String>,
    pub latency_ms: u64,
}

/// A chat model answering a system + user prompt with one JSON object. [`LlmClient`] in
/// production; tests script the answers.
#[async_trait::async_trait]
pub trait ChatModel: Send + Sync {
    /// The model requested.
    fn model(&self) -> &str;
    async fn complete_json(&self, system: &str, user: &str) -> Result<Completion, LlmError>;
}

pub struct LlmClient {
    http: reqwest::Client,
    config: LlmConfig,
}

#[async_trait::async_trait]
impl ChatModel for LlmClient {
    fn model(&self) -> &str {
        &self.config.model
    }

    async fn complete_json(&self, system: &str, user: &str) -> Result<Completion, LlmError> {
        LlmClient::complete_json(self, system, user).await
    }
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("hone-quant/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    /// The request body for a system + user prompt that must be answered with one JSON object.
    pub fn request_body(&self, system: &str, user: &str) -> Value {
        let mut body = json!({
            "model": self.config.model,
            "stream": false,
            "response_format": {"type": "json_object"},
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        if let Some(effort) = &self.config.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }
        body
    }

    pub async fn complete_json(&self, system: &str, user: &str) -> Result<Completion, LlmError> {
        let started = Instant::now();
        let response = self
            .http
            .post(format!("{}/chat/completions", self.config.base_url))
            .bearer_auth(&self.config.api_key)
            .json(&self.request_body(system, user))
            .send()
            .await
            .map_err(|e| LlmError::Transport(without_url(e)))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| LlmError::Transport(without_url(e)))?;
        if !status.is_success() {
            return Err(LlmError::Status {
                status: status.as_u16(),
                detail: truncate(&text, 600),
            });
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| LlmError::Parse(format!("not JSON: {}", truncate(&text, 200))))?;
        parse_completion(&value, started.elapsed())
    }
}

fn parse_completion(value: &Value, elapsed: Duration) -> Result<Completion, LlmError> {
    if let Some(error) = value.get("error") {
        return Err(LlmError::Parse(truncate(&error.to_string(), 600)));
    }
    let choice = value
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| LlmError::Parse("no choices".into()))?;
    let content = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if content.trim().is_empty() {
        return Err(LlmError::Parse("empty answer".into()));
    }
    Ok(Completion {
        content,
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        usage: value.get("usage").cloned().filter(|u| !u.is_null()),
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        latency_ms: elapsed.as_millis() as u64,
    })
}

/// reqwest errors embed the request URL; keep the kind of failure only.
fn without_url(error: reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else if error.is_decode() || error.is_body() {
        "could not read the answer"
    } else {
        "request error"
    };
    format!("{kind}: {}", error.without_url())
}

pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(effort: Option<&str>) -> LlmClient {
        LlmClient::new(LlmConfig {
            base_url: "http://127.0.0.1:8898/v1".into(),
            api_key: "local-proxy".into(),
            model: "gemini-3.8-flash-medium".into(),
            reasoning_effort: effort.map(str::to_string),
            timeout_secs: 60,
            origin: "test".into(),
        })
        .unwrap()
    }

    #[test]
    fn requests_are_unary_json_mode() {
        let body = client(Some("low")).request_body("sys", "user");
        assert_eq!(body["stream"], json!(false));
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "user");
        // Gemini 3 rejects sampling parameters; none are sent.
        assert!(body.get("temperature").is_none());
        assert!(
            client(None)
                .request_body("s", "u")
                .get("reasoning_effort")
                .is_none()
        );
    }

    #[test]
    fn completions_are_parsed() {
        let answer = json!({
            "model": "gemini-3.8-flash-medium",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "{\"a\":1}", "reasoning_content": "…"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        });
        let completion = parse_completion(&answer, Duration::from_millis(1200)).unwrap();
        assert_eq!(completion.content, "{\"a\":1}");
        assert_eq!(completion.finish_reason.as_deref(), Some("stop"));
        assert_eq!(completion.latency_ms, 1200);
        assert_eq!(completion.usage.unwrap()["total_tokens"], 15);

        let empty = json!({"choices": [{"message": {"content": ""}}]});
        assert!(parse_completion(&empty, Duration::ZERO).is_err());
        let error = json!({"error": {"message": "quota"}});
        assert!(parse_completion(&error, Duration::ZERO).is_err());
    }

    #[test]
    fn truncation_counts_characters() {
        assert_eq!(truncate("板块观点", 2), "板块…");
        assert_eq!(truncate("abc", 3), "abc");
    }
}
