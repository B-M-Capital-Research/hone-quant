//! A `longbridge serve` session: the client running as a long-lived child process that answers
//! newline-delimited JSON-RPC 2.0 on its standard input and output.
//!
//! hone-quant sends one request at a time (the caller holds the session's lock), so a response
//! is simply the next line carrying the request's id; anything else on the stream (quote
//! notifications, the late answer to a request that timed out) is skipped. A session that
//! exits — not signed in, a crash, the network gone — is replaced on the next call, and one
//! left idle is shut down so it does not hold Longbridge's connections open overnight.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use super::wire;
use crate::brokers::BrokerError;

/// A session unused for this long is shut down.
const IDLE: Duration = Duration::from_secs(600);
/// How long one request may take (the SDK's own HTTP timeout is 30 s).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(45);
/// Longbridge allows 30 trade calls per 30 s: one call per second at most stays inside it.
const SPACING: Duration = Duration::from_millis(1000);

struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    /// The tail of the process's standard error, for diagnosing an exit.
    stderr: Arc<StdMutex<String>>,
    next_id: u64,
    last_used: Instant,
}

impl Session {
    fn stderr_tail(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// Owns at most one running session; calls are serialised and paced.
pub struct Serve {
    session: Arc<Mutex<Option<Session>>>,
    last_call: Mutex<Option<Instant>>,
}

impl Default for Serve {
    fn default() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            last_call: Mutex::new(None),
        }
    }
}

impl Serve {
    /// Calls `method` with `params`, starting a session with `command` if none is running.
    pub async fn call(
        &self,
        command: impl FnOnce() -> Command,
        cwd: &Path,
        method: &str,
        params: Value,
    ) -> Result<Value, BrokerError> {
        let mut guard = self.session.lock().await;
        {
            let mut last = self.last_call.lock().await;
            if let Some(at) = *last {
                let wait = SPACING.saturating_sub(at.elapsed());
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            *last = Some(Instant::now());
        }
        if guard.is_none() {
            *guard = Some(start(command(), cwd)?);
            reap_when_idle(Arc::downgrade(&self.session));
        }
        let session = guard.as_mut().expect("session just started");
        session.last_used = Instant::now();
        let id = session.next_id;
        session.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let outcome = tokio::time::timeout(CALL_TIMEOUT, exchange(session, id, &request)).await;
        match outcome {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(Exchange::Rpc { code, message })) => {
                let error = wire::rpc_error(code, &message);
                if matches!(error, BrokerError::NotSignedIn(_)) {
                    // The session's token may be stale (another client process refreshed it):
                    // the next call starts a fresh session, which reads it from disk again.
                    if let Some(session) = guard.take() {
                        tokio::spawn(shutdown(session));
                    }
                }
                Err(error)
            }
            Ok(Err(Exchange::Closed)) => {
                // The process ended: the reason is on its standard error.
                let session = guard.take().expect("session present");
                Err(ended(session).await)
            }
            Err(_) => Err(BrokerError::Failed(format!(
                "`{method}` did not answer within {}s",
                CALL_TIMEOUT.as_secs()
            ))),
        }
    }
}

enum Exchange {
    Rpc { code: i64, message: String },
    Closed,
}

fn start(mut command: Command, cwd: &Path) -> Result<Session, BrokerError> {
    command
        .arg("serve")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(super::spawn_error)?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let tail = Arc::new(StdMutex::new(String::new()));
    let sink = tail.clone();
    tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buffer).await {
            if n == 0 {
                break;
            }
            if let Ok(mut text) = sink.lock() {
                text.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if text.len() > 8192 {
                    let cut = text.len() - 4096;
                    let cut = (cut..text.len())
                        .find(|i| text.is_char_boundary(*i))
                        .unwrap_or(text.len());
                    text.drain(..cut);
                }
            }
        }
    });
    tracing::info!("longbridge serve session started");
    Ok(Session {
        child,
        stdin,
        lines: BufReader::new(stdout).lines(),
        stderr: tail,
        next_id: 1,
        last_used: Instant::now(),
    })
}

async fn exchange(session: &mut Session, id: u64, request: &Value) -> Result<Value, Exchange> {
    let mut line = request.to_string();
    line.push('\n');
    if session.stdin.write_all(line.as_bytes()).await.is_err()
        || session.stdin.flush().await.is_err()
    {
        return Err(Exchange::Closed);
    }
    loop {
        let Ok(Some(line)) = session.lines.next_line().await else {
            return Err(Exchange::Closed);
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!(line = %line, "longbridge serve: ignoring a line that is not JSON");
            continue;
        };
        if message["id"].as_u64() != Some(id) {
            // A quote notification, or the answer to an earlier request that timed out.
            continue;
        }
        if let Some(error) = message.get("error").filter(|e| !e.is_null()) {
            return Err(Exchange::Rpc {
                code: error["code"].as_i64().unwrap_or(0),
                message: error["message"].as_str().unwrap_or_default().to_string(),
            });
        }
        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
    }
}

/// The error for a session whose process ended.
async fn ended(mut session: Session) -> BrokerError {
    let status = tokio::time::timeout(Duration::from_secs(2), session.child.wait()).await;
    // Give the stderr reader a moment to collect the last words.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let stderr = session.stderr_tail();
    let code = status.ok().and_then(|s| s.ok()).and_then(|s| s.code());
    tracing::warn!(?code, stderr = %stderr.trim(), "longbridge serve session ended");
    if stderr.trim().is_empty() {
        BrokerError::Failed(format!(
            "the Longbridge client stopped unexpectedly (exit code {code:?})"
        ))
    } else {
        wire::classify_failure(&stderr, "")
    }
}

async fn shutdown(mut session: Session) {
    let id = session.next_id;
    let request = json!({"jsonrpc": "2.0", "id": id, "method": "shutdown"});
    let _ =
        tokio::time::timeout(Duration::from_secs(5), exchange(&mut session, id, &request)).await;
    drop(session.stdin);
    if tokio::time::timeout(Duration::from_secs(5), session.child.wait())
        .await
        .is_err()
    {
        let _ = session.child.kill().await;
    }
    tracing::info!("longbridge serve session stopped");
}

/// Shuts the session down once it has been idle for [`IDLE`].
fn reap_when_idle(session: Weak<Mutex<Option<Session>>>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let Some(slot) = session.upgrade() else {
                return;
            };
            let Ok(mut guard) = slot.try_lock() else {
                continue;
            };
            let idle = match guard.as_ref() {
                None => return,
                Some(s) => s.last_used.elapsed() >= IDLE,
            };
            if idle {
                let s = guard.take().expect("checked");
                drop(guard);
                shutdown(s).await;
                return;
            }
        }
    });
}
