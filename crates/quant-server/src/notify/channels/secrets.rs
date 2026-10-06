//! Masking, restoring and scrubbing channel secrets.
//!
//! The settings UI never receives a stored secret: [`redacted`] keeps only enough to recognise
//! which credential is configured (a webhook's host, the last four characters). When a form is
//! saved without touching those fields the masks come straight back, and [`merge_secrets`] swaps
//! them for the stored originals instead of overwriting working credentials with bullets.

use std::cmp::Reverse;

use super::{ChannelConfig, ChannelError};

/// Marks a masked value: it starts every masked secret and follows the host in a masked URL.
pub const MASK: &str = "••••";

/// Trailing characters a mask leaves visible.
const VISIBLE_TAIL: usize = 4;
/// Path segments and query values at least this long are scrubbed from errors on their own:
/// webhook tokens are long and random, while short segments are words like `hook` or `send`.
const MIN_URL_TOKEN: usize = 8;

/// The configuration with every secret masked, for API responses and logs.
///
/// A secret becomes `••••` plus its last four characters (just `••••` when it has no more than
/// four). A URL keeps its scheme and host and masks the rest — `https://host/••••abcd` — so the
/// user can still tell which webhook is configured; a URL with nothing after the host is kept as
/// is. Empty values stay empty: there is nothing to hide, and an empty secret means "not set".
pub fn redacted(config: &ChannelConfig) -> ChannelConfig {
    let mut config = config.clone();
    match &mut config {
        ChannelConfig::Telegram { bot_token, .. } => *bot_token = mask(bot_token),
        ChannelConfig::Feishu {
            webhook_url,
            secret,
        } => {
            *webhook_url = mask_url(webhook_url);
            mask_optional(secret);
        }
        ChannelConfig::Wecom { webhook_url }
        | ChannelConfig::Slack { webhook_url }
        | ChannelConfig::Discord { webhook_url } => *webhook_url = mask_url(webhook_url),
        ChannelConfig::Webhook { url, secret } => {
            *url = mask_url(url);
            mask_optional(secret);
        }
        ChannelConfig::Email { password, .. } => mask_optional(password),
    }
    config
}

/// Restores secrets that come back masked from the settings UI.
///
/// Each secret field of `incoming` that still holds a mask is replaced by the same field of
/// `existing`, provided both are the same kind of channel; every other field — including a
/// secret the user retyped or cleared — comes from `incoming`. A mask that cannot be restored
/// (a new channel, or a changed kind) is left in place for [`validate`](super::validate()) to
/// reject.
///
/// The restored secret follows whatever else was edited, so anyone allowed to edit a channel can
/// point its stored credential at a new destination (an SMTP password at another host, say).
/// That is fine while channel settings are admin-only; a multi-user deployment should require
/// re-entering secrets when the destination changes.
pub fn merge_secrets(incoming: ChannelConfig, existing: Option<&ChannelConfig>) -> ChannelConfig {
    use ChannelConfig as C;

    let Some(existing) = existing else {
        return incoming;
    };
    match (incoming, existing) {
        (
            C::Telegram { bot_token, chat_id },
            C::Telegram {
                bot_token: kept, ..
            },
        ) => C::Telegram {
            bot_token: restore(bot_token, kept),
            chat_id,
        },
        (
            C::Feishu {
                webhook_url,
                secret,
            },
            C::Feishu {
                webhook_url: kept_url,
                secret: kept_secret,
            },
        ) => C::Feishu {
            webhook_url: restore(webhook_url, kept_url),
            secret: restore_optional(secret, kept_secret),
        },
        (C::Wecom { webhook_url }, C::Wecom { webhook_url: kept }) => C::Wecom {
            webhook_url: restore(webhook_url, kept),
        },
        (C::Slack { webhook_url }, C::Slack { webhook_url: kept }) => C::Slack {
            webhook_url: restore(webhook_url, kept),
        },
        (C::Discord { webhook_url }, C::Discord { webhook_url: kept }) => C::Discord {
            webhook_url: restore(webhook_url, kept),
        },
        (
            C::Webhook { url, secret },
            C::Webhook {
                url: kept_url,
                secret: kept_secret,
            },
        ) => C::Webhook {
            url: restore(url, kept_url),
            secret: restore_optional(secret, kept_secret),
        },
        (
            C::Email {
                host,
                port,
                username,
                password,
                from,
                to,
                tls,
            },
            C::Email { password: kept, .. },
        ) => C::Email {
            host,
            port,
            username,
            password: restore_optional(password, kept),
            from,
            to,
            tls,
        },
        (incoming, _) => incoming,
    }
}

fn restore(incoming: String, kept: &str) -> String {
    if is_masked(&incoming) {
        kept.to_string()
    } else {
        incoming
    }
}

fn restore_optional(incoming: Option<String>, kept: &Option<String>) -> Option<String> {
    match incoming {
        Some(value) if is_masked(&value) => kept.clone(),
        incoming => incoming,
    }
}

/// Whether `value` carries a mask produced by [`redacted`]. `contains` rather than `starts_with`,
/// because in a masked URL the marker follows the host.
pub(super) fn is_masked(value: &str) -> bool {
    value.contains(MASK)
}

/// `err` with every secret of `config` replaced by [`MASK`]. Errors can quote a server's reply,
/// and servers echo what they were sent — Express answers `Cannot POST /hooks/<token>`.
pub(super) fn scrub(err: ChannelError, config: &ChannelConfig) -> ChannelError {
    let mut secrets = secret_values(config);
    // Longest first, so a whole URL is replaced before the path inside it.
    secrets.sort_by_key(|secret| Reverse(secret.len()));
    let clean = |text: String| {
        secrets
            .iter()
            .fold(text, |text, secret| text.replace(secret.as_str(), MASK))
    };
    match err {
        ChannelError::Config(text) => ChannelError::Config(clean(text)),
        ChannelError::Delivery(text) => ChannelError::Delivery(clean(text)),
    }
}

/// Every string worth scrubbing. Values under four characters cannot be real credentials, and
/// replacing them would garble the message.
fn secret_values(config: &ChannelConfig) -> Vec<String> {
    let mut values = Vec::new();
    match config {
        ChannelConfig::Telegram { bot_token, .. } => values.push(bot_token.trim().to_string()),
        ChannelConfig::Feishu {
            webhook_url,
            secret,
        } => {
            push_url_parts(&mut values, webhook_url);
            values.extend(secret.clone());
        }
        ChannelConfig::Wecom { webhook_url }
        | ChannelConfig::Slack { webhook_url }
        | ChannelConfig::Discord { webhook_url } => push_url_parts(&mut values, webhook_url),
        ChannelConfig::Webhook { url, secret } => {
            push_url_parts(&mut values, url);
            values.extend(secret.clone());
        }
        ChannelConfig::Email { password, .. } => values.extend(password.clone()),
    }
    values.retain(|value| value.chars().count() >= VISIBLE_TAIL);
    values
}

/// The URL, its credentials, its path and query (with the leading `/`, so a bare word never
/// matches), and the token-like pieces of those: the last path segment and query values.
fn push_url_parts(values: &mut Vec<String>, url: &str) {
    let url = url.trim();
    values.push(url.to_string());
    let Some(parts) = split_url(url) else {
        return;
    };
    values.extend(parts.userinfo.map(str::to_string));
    if parts.rest.len() > 1 {
        values.push(parts.rest.to_string());
    }
    let (path, query) = parts.rest.split_once('?').unwrap_or((parts.rest, ""));
    let last_segment = path.rsplit('/').next();
    let query_values = query
        .split('&')
        .filter_map(|pair| pair.split_once('=').map(|kv| kv.1));
    values.extend(
        last_segment
            .into_iter()
            .chain(query_values)
            .filter(|token| token.chars().count() >= MIN_URL_TOKEN)
            .map(str::to_string),
    );
}

/// `••••` plus the last four characters, or just `••••` for four characters or fewer.
fn mask(secret: &str) -> String {
    let count = secret.chars().count();
    match count {
        0 => String::new(),
        1..=VISIBLE_TAIL => MASK.to_string(),
        _ => {
            let tail: String = secret.chars().skip(count - VISIBLE_TAIL).collect();
            format!("{MASK}{tail}")
        }
    }
}

fn mask_optional(secret: &mut Option<String>) {
    if let Some(value) = secret {
        *value = mask(value);
    }
}

/// Scheme and host stay readable; credentials, path and query — where webhook tokens live — are
/// masked: `https://hooks.slack.com/services/T0/B0/XYZ1234` → `https://hooks.slack.com/••••1234`.
fn mask_url(url: &str) -> String {
    let Some(parts) = split_url(url) else {
        return mask(url);
    };
    let path = parts.rest.trim_start_matches('/');
    let hidden = if path.is_empty() {
        parts.userinfo.unwrap_or_default()
    } else {
        path
    };
    if hidden.is_empty() {
        return url.to_string();
    }
    format!("{}://{}/{}", parts.scheme, parts.host, mask(hidden))
}

/// `scheme://[userinfo@]host[:port]` and everything after it, split without normalising.
struct UrlParts<'a> {
    scheme: &'a str,
    userinfo: Option<&'a str>,
    host: &'a str,
    /// Path, query and fragment, starting at the first `/`, `?` or `#` (or empty).
    rest: &'a str,
}

fn split_url(url: &str) -> Option<UrlParts<'_>> {
    let (scheme, after) = url.split_once("://")?;
    let (authority, rest) = after.split_at(after.find(['/', '?', '#']).unwrap_or(after.len()));
    let (userinfo, host) = match authority.rsplit_once('@') {
        Some((userinfo, host)) => (Some(userinfo), host),
        None => (None, authority),
    };
    Some(UrlParts {
        scheme,
        userinfo,
        host,
        rest,
    })
}

#[cfg(test)]
mod tests {
    use super::super::testkit::samples;
    use super::super::{EmailTls, kind, validate};
    use super::*;

    #[test]
    fn every_secret_is_masked_to_its_last_four_characters() {
        let masked: Vec<ChannelConfig> = samples().iter().map(redacted).collect();
        let expected = [
            ChannelConfig::Telegram {
                bot_token: "••••oken".into(),
                chat_id: "-100200300".into(),
            },
            ChannelConfig::Feishu {
                webhook_url: "https://open.feishu.cn/••••oken".into(),
                secret: Some("••••cret".into()),
            },
            ChannelConfig::Wecom {
                webhook_url: "https://qyapi.weixin.qq.com/••••0123".into(),
            },
            ChannelConfig::Slack {
                webhook_url: "https://hooks.slack.com/••••xyz9".into(),
            },
            ChannelConfig::Discord {
                webhook_url: "https://discord.com/••••abcd".into(),
            },
            ChannelConfig::Webhook {
                url: "https://hooks.example.com/••••5678".into(),
                secret: Some("••••ning".into()),
            },
            ChannelConfig::Email {
                host: "smtp.example.com".into(),
                port: 587,
                username: Some("alerts@example.com".into()),
                password: Some("••••1234".into()),
                from: "hone-quant <alerts@example.com>".into(),
                to: vec!["ops@example.com".into()],
                tls: EmailTls::StartTls,
            },
        ];
        assert_eq!(masked, expected);
    }

    #[test]
    fn no_secret_survives_redaction() {
        // Distinctive pieces of every sample secret, minus the tail a mask may show.
        let leaks = [
            "ABC-telegram",
            "feishu-hook",
            "feishu-signing",
            "open-apis",
            "wecom-key",
            "key=",
            "T0001",
            "slack-secret",
            "discord-token",
            "webhooks/123",
            "receiver-token",
            "whsec-sig",
            "smtp-password",
        ];
        for config in samples() {
            let json = serde_json::to_string(&redacted(&config)).unwrap();
            for leak in leaks {
                assert!(
                    !json.contains(leak),
                    "{} leaks {leak:?}: {json}",
                    kind(&config)
                );
            }
        }
    }

    #[test]
    fn short_and_empty_secrets() {
        assert_eq!(mask("abcd"), MASK);
        assert_eq!(mask("密钥"), MASK);
        assert_eq!(mask("abcde"), "••••bcde");
        assert_eq!(mask("交易密钥一二三四"), "••••一二三四");
        assert_eq!(mask(""), "");
        let config = ChannelConfig::Webhook {
            url: "https://hooks.example.com".into(),
            secret: Some(String::new()),
        };
        assert_eq!(redacted(&config), config);
    }

    #[test]
    fn urls_keep_scheme_and_host_only() {
        assert_eq!(
            mask_url("https://hooks.example.com/"),
            "https://hooks.example.com/"
        );
        assert_eq!(
            mask_url("http://127.0.0.1:8080/in/abcdefgh"),
            "http://127.0.0.1:8080/••••efgh"
        );
        assert_eq!(
            mask_url("https://hooks.example.com?token=abc"),
            "https://hooks.example.com/••••=abc"
        );
        assert_eq!(
            mask_url("https://user:pa55word@hooks.example.com"),
            "https://hooks.example.com/••••word"
        );
        assert_eq!(
            mask_url("https://user:pw@hooks.example.com/x/abcdef"),
            "https://hooks.example.com/••••cdef"
        );
        assert_eq!(mask_url("hooks.example.com/abcdef"), "••••cdef");
    }

    #[test]
    fn redaction_is_idempotent() {
        for config in samples() {
            let once = redacted(&config);
            assert_eq!(redacted(&once), once);
        }
    }

    #[test]
    fn masked_secrets_are_restored_from_the_stored_config() {
        for config in samples() {
            assert_eq!(merge_secrets(redacted(&config), Some(&config)), config);
        }
    }

    #[test]
    fn new_secrets_and_other_fields_come_from_the_incoming_config() {
        for config in samples() {
            assert_eq!(merge_secrets(config.clone(), Some(&samples()[0])), config);
        }
        let stored = ChannelConfig::Telegram {
            bot_token: "111:old-token-aaaa".into(),
            chat_id: "1".into(),
        };
        let incoming = ChannelConfig::Telegram {
            bot_token: "222:new-token-bbbb".into(),
            chat_id: "2".into(),
        };
        assert_eq!(merge_secrets(incoming.clone(), Some(&stored)), incoming);

        let stored = samples().pop().unwrap();
        let ChannelConfig::Email { password, .. } = &stored else {
            unreachable!()
        };
        let ChannelConfig::Email {
            port,
            username,
            from,
            tls,
            ..
        } = redacted(&stored)
        else {
            unreachable!()
        };
        let incoming = ChannelConfig::Email {
            host: "smtp.other.example".into(),
            port,
            username,
            password: Some("••••1234".into()),
            from,
            to: vec!["a@example.com".into(), "b@example.com".into()],
            tls,
        };
        let merged = merge_secrets(incoming, Some(&stored));
        let ChannelConfig::Email {
            host,
            to,
            password: merged_password,
            ..
        } = merged
        else {
            unreachable!()
        };
        assert_eq!(host, "smtp.other.example");
        assert_eq!(to.len(), 2);
        assert_eq!(&merged_password, password);
    }

    #[test]
    fn fields_are_restored_independently() {
        let stored = ChannelConfig::Feishu {
            webhook_url: "https://open.feishu.cn/open-apis/bot/v2/hook/old-hook-token".into(),
            secret: Some("old-signing-secret".into()),
        };
        let incoming = ChannelConfig::Feishu {
            webhook_url: "https://open.feishu.cn/••••oken".into(),
            secret: Some("new-signing-secret".into()),
        };
        let expected = ChannelConfig::Feishu {
            webhook_url: "https://open.feishu.cn/open-apis/bot/v2/hook/old-hook-token".into(),
            secret: Some("new-signing-secret".into()),
        };
        assert_eq!(merge_secrets(incoming, Some(&stored)), expected);

        // Clearing an optional secret clears it.
        let stored = samples()[5].clone();
        let ChannelConfig::Webhook { url, .. } = redacted(&stored) else {
            unreachable!()
        };
        let merged = merge_secrets(ChannelConfig::Webhook { url, secret: None }, Some(&stored));
        let ChannelConfig::Webhook { url, secret } = merged else {
            unreachable!()
        };
        assert_eq!(url, "https://hooks.example.com/in/receiver-token-5678");
        assert_eq!(secret, None);
    }

    #[test]
    fn a_kind_change_or_new_channel_keeps_the_incoming_masks() {
        let stored = samples()[3].clone(); // Slack
        let incoming = ChannelConfig::Discord {
            webhook_url: "https://discord.com/••••xyz9".into(),
        };
        let merged = merge_secrets(incoming.clone(), Some(&stored));
        assert_eq!(merged, incoming);
        assert!(validate(&merged).is_err());

        let incoming = redacted(&samples()[0]);
        assert_eq!(merge_secrets(incoming.clone(), None), incoming);
    }

    #[test]
    fn scrub_removes_secrets_and_url_tokens_from_errors() {
        let config = ChannelConfig::Webhook {
            url: "https://hooks.example.com/in/receiver-token-5678?sig=query-secret-99".into(),
            secret: Some("whsec-signing".into()),
        };
        let err = ChannelError::Delivery(
            "Cannot POST /in/receiver-token-5678?sig=query-secret-99 \
             (token receiver-token-5678, sig query-secret-99, key whsec-signing, \
             url https://hooks.example.com/in/receiver-token-5678?sig=query-secret-99)"
                .into(),
        );
        let text = scrub(err, &config).to_string();
        for secret in ["receiver-token", "query-secret", "whsec-signing", "/in/"] {
            assert!(!text.contains(secret), "{secret:?} in {text}");
        }
        assert!(
            text.starts_with("delivery failed: Cannot POST •••• (token ••••"),
            "{text}"
        );

        let config = ChannelConfig::Email {
            host: "smtp.example.com".into(),
            port: 587,
            username: Some("alerts".into()),
            password: Some("hunter2-password".into()),
            from: "alerts@example.com".into(),
            to: vec!["ops@example.com".into()],
            tls: EmailTls::StartTls,
        };
        let err = ChannelError::Config("535 bad password hunter2-password".into());
        assert_eq!(
            scrub(err, &config).to_string(),
            "invalid channel configuration: 535 bad password ••••"
        );
    }

    #[test]
    fn scrub_leaves_ordinary_words_alone() {
        let config = ChannelConfig::Webhook {
            url: "https://example.com/webhook".into(),
            secret: Some("abc".into()),
        };
        let err = ChannelError::Delivery("webhook says: abc not found".into());
        assert_eq!(
            scrub(err, &config).to_string(),
            "delivery failed: webhook says: abc not found"
        );
    }
}
