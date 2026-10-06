//! SMTP email through lettre, as `multipart/alternative`: a plain-text part and a small
//! inline-styled HTML part.
//!
//! The plain part is what watches, screen readers and strict corporate gateways show, so it
//! carries everything; the HTML part only adds a title, a severity colour and a button. Styles are
//! inline because most mail clients drop `<style>` blocks.

use std::time::Duration;

use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::format::Escape;
use super::{ChannelError, EmailTls, OutboundMessage, REQUEST_TIMEOUT};

/// lettre applies [`REQUEST_TIMEOUT`] to each SMTP command; this bounds the whole session
/// (connect, TLS, EHLO, AUTH, MAIL, RCPT…, DATA).
const SESSION_TIMEOUT: Duration = Duration::from_secs(30);
const LINK_TEXT: &str = "打开 hone-quant / Open";

const PAGE_STYLE: &str = "margin:0;padding:24px 12px;background:#f4f5f7;";
const CARD_STYLE: &str = "max-width:560px;margin:0 auto;padding:20px 24px;background:#ffffff;\
    border-radius:8px;font-size:14px;line-height:1.6;color:#1f2328;font-family:-apple-system,\
    BlinkMacSystemFont,'Segoe UI',Roboto,'PingFang SC','Microsoft YaHei',sans-serif;";
const TITLE_STYLE: &str = "margin:0 0 12px;font-size:18px;line-height:1.4;";
const BUTTON_STYLE: &str = "display:inline-block;padding:8px 16px;border-radius:6px;\
    background:#1f2328;color:#ffffff;text-decoration:none;";
const FOOTER_STYLE: &str = "margin:20px 0 0;font-size:12px;color:#6b7280;";

/// Where and how to submit mail.
pub(super) struct Smtp<'a> {
    pub host: &'a str,
    pub port: u16,
    /// Authenticate only when set.
    pub username: Option<&'a str>,
    pub password: Option<&'a str>,
    pub tls: EmailTls,
}

pub(super) async fn send(
    smtp: &Smtp<'_>,
    from: &str,
    to: &[String],
    message: &OutboundMessage,
) -> Result<(), ChannelError> {
    let email = build_message(from, to, message)?;
    let transport = transport(smtp)?;
    match tokio::time::timeout(SESSION_TIMEOUT, transport.send(email)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(ChannelError::Delivery(format!("SMTP: {err}"))),
        Err(_) => Err(ChannelError::Delivery(format!(
            "SMTP session did not finish within {}s",
            SESSION_TIMEOUT.as_secs()
        ))),
    }
}

/// STARTTLS and implicit TLS verify the server certificate against `host`; `None` is plain text
/// and only fit for a relay on a trusted network.
fn transport(smtp: &Smtp<'_>) -> Result<AsyncSmtpTransport<Tokio1Executor>, ChannelError> {
    let builder = match smtp.tls {
        EmailTls::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp.host),
        EmailTls::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(smtp.host),
        EmailTls::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            smtp.host,
        )),
    }
    .map_err(|err| ChannelError::Config(format!("SMTP host is unusable for TLS: {err}")))?;
    let mut builder = builder.port(smtp.port).timeout(Some(REQUEST_TIMEOUT));
    if let Some(username) = smtp.username {
        let password = smtp.password.unwrap_or_default();
        builder = builder.credentials(Credentials::new(username.into(), password.into()));
    }
    Ok(builder.build())
}

/// The email as submitted: subject, sender, recipients and both bodies. Pure apart from the
/// generated `Message-ID` and `Date`.
pub(super) fn build_message(
    from: &str,
    to: &[String],
    message: &OutboundMessage,
) -> Result<Message, ChannelError> {
    let sender = mailbox("sender", from)?;
    // Spam filters penalise a missing Message-ID, and one in the sender's domain looks native.
    let message_id = format!("<{}@{}>", uuid::Uuid::new_v4(), sender.email.domain());
    let mut builder = Message::builder()
        .from(sender)
        .subject(subject(message))
        .message_id(Some(message_id));
    for recipient in to {
        builder = builder.to(mailbox("recipient", recipient)?);
    }
    let bodies = render(message);
    builder
        .multipart(MultiPart::alternative_plain_html(bodies.text, bodies.html))
        .map_err(|err| ChannelError::Config(format!("cannot build the email: {err}")))
}

fn mailbox(role: &str, address: &str) -> Result<Mailbox, ChannelError> {
    address
        .trim()
        .parse()
        .map_err(|_| ChannelError::Config(format!("invalid {role} address {address:?}")))
}

/// `[hone-quant] {title}` on one line: a line break would end the header early.
fn subject(message: &OutboundMessage) -> String {
    let title: String = message
        .title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("[hone-quant] {title}")
}

/// Both bodies of the email.
struct Bodies {
    text: String,
    html: String,
}

fn render(message: &OutboundMessage) -> Bodies {
    let footer = format!(
        "hone-quant · {} · {}",
        message.category,
        message.timestamp.format("%Y-%m-%d %H:%M UTC")
    );
    let body = message.body.trim_end();
    let link = message.web_link();

    let mut sections = vec![message.title.trim().to_string()];
    if !body.is_empty() {
        sections.push(body.to_string());
    }
    if let Some(link) = link {
        sections.push(format!("{LINK_TEXT}: {link}"));
    }
    sections.push(format!("-- \n{footer}"));
    let text = sections.join("\n\n") + "\n";

    let accent = format!("#{:06X}", message.severity.color());
    let title = Escape::HtmlAttr.apply(message.title.trim());
    let body = Escape::HtmlAttr.apply(body).replace('\n', "<br>\n");
    let button = link
        .map(|link| {
            let href = Escape::HtmlAttr.apply(link);
            let anchor = format!(r#"<a href="{href}" style="{BUTTON_STYLE}">{LINK_TEXT}</a>"#);
            format!("<p style=\"margin:20px 0 0;\">{anchor}</p>\n")
        })
        .unwrap_or_default();
    let footer = Escape::HtmlAttr.apply(&footer);
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
</head>
<body style="{PAGE_STYLE}">
<div style="{CARD_STYLE}border-top:4px solid {accent};">
<h1 style="{TITLE_STYLE}">{title}</h1>
<div>{body}</div>
{button}<p style="{FOOTER_STYLE}">{footer}</p>
</div>
</body>
</html>
"#
    );
    Bodies { text, html }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::super::testkit::{client, message};
    use super::super::{ChannelConfig, send};
    use super::*;

    fn recipients() -> Vec<String> {
        vec![
            "ops@example.com".into(),
            "Risk Desk <risk@example.org>".into(),
        ]
    }

    #[test]
    fn message_has_subject_sender_recipients_and_both_parts() {
        let email =
            build_message("hone-quant <alerts@example.com>", &recipients(), &message()).unwrap();
        let headers = email.headers();
        assert_eq!(
            headers.get_raw("Subject"),
            Some("[hone-quant] Plan <ready> & waiting")
        );
        assert!(
            headers
                .get_raw("From")
                .unwrap()
                .contains("alerts@example.com")
        );
        assert!(
            headers
                .get_raw("Message-ID")
                .unwrap()
                .ends_with("@example.com>")
        );

        let envelope = email.envelope();
        assert_eq!(envelope.from().unwrap().to_string(), "alerts@example.com");
        let to: Vec<String> = envelope.to().iter().map(ToString::to_string).collect();
        assert_eq!(to, ["ops@example.com", "risk@example.org"]);

        let formatted = String::from_utf8(email.formatted()).unwrap();
        assert!(formatted.contains("Content-Type: multipart/alternative"));
        let plain = formatted
            .find("Content-Type: text/plain; charset=utf-8")
            .unwrap();
        let html = formatted
            .find("Content-Type: text/html; charset=utf-8")
            .unwrap();
        assert!(plain < html, "the preferred (HTML) alternative comes last");
    }

    #[test]
    fn plain_text_carries_title_body_link_and_footer() {
        assert_eq!(
            render(&message()).text,
            "Plan <ready> & waiting\n\n\
             买入 AAPL 10 股 @ 180.00\nSell MSFT if P/E > 35 & RSI < 30\n\n\
             打开 hone-quant / Open: https://quant.example.com/plans/42?slot=open&mode=review\n\n\
             -- \nhone-quant · rebalance · 2026-10-05 13:30 UTC\n"
        );
    }

    #[test]
    fn html_is_escaped_and_styled_inline() {
        let html = render(&message()).html;
        let heading = format!("<h1 style=\"{TITLE_STYLE}\">Plan &lt;ready&gt; &amp; waiting</h1>");
        assert!(html.contains(&heading));
        assert!(html.contains(
            "<div>买入 AAPL 10 股 @ 180.00<br>\nSell MSFT if P/E &gt; 35 &amp; RSI &lt; 30</div>"
        ));
        assert!(
            html.contains("href=\"https://quant.example.com/plans/42?slot=open&amp;mode=review\"")
        );
        assert!(html.contains("border-top:4px solid #A37B12"));
        assert!(!html.contains("<ready>") && !html.contains("<style"));
    }

    #[test]
    fn no_link_means_no_button() {
        let message = OutboundMessage {
            link: None,
            ..message()
        };
        let bodies = render(&message);
        assert!(!bodies.html.contains("<a ") && !bodies.text.contains(LINK_TEXT));
    }

    #[test]
    fn subject_stays_on_one_line() {
        let message = OutboundMessage {
            title: "Fill\r\nBcc: victim@example.com\t done".into(),
            ..message()
        };
        let email = build_message("alerts@example.com", &recipients(), &message).unwrap();
        assert_eq!(
            email.headers().get_raw("Subject"),
            Some("[hone-quant] Fill Bcc: victim@example.com done")
        );
        assert_eq!(email.envelope().to().len(), 2);
    }

    #[test]
    fn invalid_addresses_are_configuration_errors() {
        let err = build_message("alerts", &recipients(), &message()).unwrap_err();
        assert!(
            matches!(err, ChannelError::Config(ref m) if m.contains("sender")),
            "{err}"
        );
        let err = build_message("alerts@example.com", &["ops@".into()], &message()).unwrap_err();
        assert!(
            matches!(err, ChannelError::Config(ref m) if m.contains("recipient")),
            "{err}"
        );
    }

    /// A one-connection SMTP server that accepts everything except AUTH, which it answers with
    /// `auth_reply`. Resolves to the client's side of the conversation.
    async fn fake_smtp(auth_reply: &'static str) -> (u16, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            let mut transcript = Vec::new();
            let mut in_data = false;
            write.write_all(b"220 fake.local ESMTP\r\n").await.unwrap();
            while let Some(line) = lines.next_line().await.unwrap() {
                transcript.push(line.clone());
                if in_data {
                    if line == "." {
                        in_data = false;
                        write.write_all(b"250 2.0.0 queued\r\n").await.unwrap();
                    }
                    continue;
                }
                let verb = line
                    .split(' ')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                let reply: &str = match verb.as_str() {
                    "EHLO" => "250-fake.local\r\n250 AUTH PLAIN LOGIN\r\n",
                    "AUTH" => auth_reply,
                    "MAIL" | "RCPT" => "250 2.1.0 ok\r\n",
                    "DATA" => {
                        in_data = true;
                        "354 end with <CRLF>.<CRLF>\r\n"
                    }
                    "QUIT" => "221 2.0.0 bye\r\n",
                    _ => "502 5.5.2 unsupported\r\n",
                };
                write.write_all(reply.as_bytes()).await.unwrap();
                if verb == "QUIT" {
                    break;
                }
            }
            transcript
        });
        (port, server)
    }

    fn plain_smtp(port: u16, password: &str) -> ChannelConfig {
        ChannelConfig::Email {
            host: "127.0.0.1".into(),
            port,
            username: Some("alerts".into()),
            password: Some(password.into()),
            from: "hone-quant <alerts@example.com>".into(),
            to: recipients(),
            tls: EmailTls::None,
        }
    }

    #[tokio::test]
    async fn delivers_over_smtp_with_authentication() {
        let (port, server) = fake_smtp("235 2.7.0 accepted\r\n").await;
        let message = OutboundMessage {
            title: "Orders filled".into(),
            ..message()
        };
        send(&client(), &plain_smtp(port, "smtp-password"), &message)
            .await
            .unwrap();

        let transcript = server.await.unwrap();
        assert!(transcript[0].starts_with("EHLO "));
        assert!(
            transcript
                .iter()
                .any(|line| line.starts_with("AUTH PLAIN "))
        );
        assert!(transcript.contains(&"MAIL FROM:<alerts@example.com>".to_string()));
        assert!(transcript.contains(&"RCPT TO:<ops@example.com>".to_string()));
        assert!(transcript.contains(&"RCPT TO:<risk@example.org>".to_string()));
        assert!(transcript.contains(&"Subject: [hone-quant] Orders filled".to_string()));
        assert!(transcript.iter().any(|line| line.contains("text/plain")));
        assert!(transcript.iter().any(|line| line.contains("text/html")));
        assert_eq!(transcript.last().map(String::as_str), Some("QUIT"));
    }

    #[tokio::test]
    async fn rejected_login_is_reported_without_the_password() {
        let (port, server) =
            fake_smtp("535 5.7.8 credentials rejected: hunter2-password\r\n").await;
        let err = send(&client(), &plain_smtp(port, "hunter2-password"), &message())
            .await
            .unwrap_err();
        server.await.unwrap();
        let text = err.to_string();
        assert!(matches!(err, ChannelError::Delivery(_)));
        assert!(
            text.contains("535") && text.contains("credentials rejected: ••••"),
            "{text}"
        );
        assert!(!text.contains("hunter2"), "{text}");
    }
}
