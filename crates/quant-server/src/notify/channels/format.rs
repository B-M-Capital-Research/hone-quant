//! Text shaping shared by the channels: escaping, and cutting text to a platform's length limit.
//!
//! Escaping and measuring happen together because the limit applies to what is sent: `&` costs
//! five bytes once it is `&amp;`, and a cut that lands inside an entity makes Telegram reject the
//! whole message.

/// Appended where text was cut.
pub(super) const ELLIPSIS: &str = "…";

/// How a platform counts toward its length limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unit {
    /// UTF-8 bytes (WeCom, Feishu).
    Bytes,
    /// UTF-16 code units, for limits documented in "characters" (Telegram, Slack, Discord).
    /// Platforms disagree on whether a character is a code point or a UTF-16 unit; the UTF-16
    /// count is never the smaller, so text that fits by it fits by either.
    Utf16,
}

impl Unit {
    pub(super) fn measure(self, text: &str) -> usize {
        match self {
            Unit::Bytes => text.len(),
            Unit::Utf16 => text.encode_utf16().count(),
        }
    }
}

/// Per-character rewriting for a target format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Escape {
    /// Sent as is (JSON string fields, plain-text formats).
    Plain,
    /// `&`, `<` and `>` as entities: the markup characters of Telegram HTML and Slack mrkdwn.
    Html,
    /// [`Escape::Html`] plus both quotes, safe inside HTML attribute values.
    HtmlAttr,
    /// Markdown block quote: every new line continues the quote with `> `.
    Quote,
}

impl Escape {
    fn replacement(self, ch: char) -> Option<&'static str> {
        match (self, ch) {
            (Escape::Html | Escape::HtmlAttr, '&') => Some("&amp;"),
            (Escape::Html | Escape::HtmlAttr, '<') => Some("&lt;"),
            (Escape::Html | Escape::HtmlAttr, '>') => Some("&gt;"),
            (Escape::HtmlAttr, '"') => Some("&quot;"),
            (Escape::HtmlAttr, '\'') => Some("&#39;"),
            (Escape::Quote, '\n') => Some("\n> "),
            _ => None,
        }
    }

    /// `text` with every character rewritten.
    pub(super) fn apply(self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            match self.replacement(ch) {
                Some(replacement) => out.push_str(replacement),
                None => out.push(ch),
            }
        }
        out
    }
}

/// `text`, escaped, in at most `max` units; text that does not fit is cut and ends in `…`.
///
/// Cuts fall between source characters, so they never split a multi-byte character or an escape
/// such as `&amp;`.
pub(super) fn fit(text: &str, max: usize, unit: Unit, escape: Escape) -> String {
    let ellipsis = unit.measure(ELLIPSIS);
    let mut out = String::new();
    let mut used = 0;
    // Length of the longest prefix of `out` that still leaves room for the ellipsis.
    let mut cut = 0;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        let piece = match escape.replacement(ch) {
            Some(replacement) => replacement,
            None => &*ch.encode_utf8(&mut buf),
        };
        let len = unit.measure(piece);
        if used + len > max {
            out.truncate(cut);
            if ellipsis <= max {
                out.push_str(ELLIPSIS);
            }
            return out;
        }
        out.push_str(piece);
        used += len;
        if used + ellipsis <= max {
            cut = out.len();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escapes_markup_and_attr_also_quotes() {
        let text = r#"P/E < 35 && "cheap" > 'fair'"#;
        assert_eq!(
            Escape::Html.apply(text),
            r#"P/E &lt; 35 &amp;&amp; "cheap" &gt; 'fair'"#
        );
        assert_eq!(
            Escape::HtmlAttr.apply(text),
            "P/E &lt; 35 &amp;&amp; &quot;cheap&quot; &gt; &#39;fair&#39;"
        );
        assert_eq!(Escape::Plain.apply(text), text);
    }

    #[test]
    fn quote_continues_on_every_line() {
        assert_eq!(Escape::Quote.apply("买入\n卖出\n"), "买入\n> 卖出\n> ");
    }

    #[test]
    fn text_that_fits_is_only_escaped() {
        assert_eq!(fit("a&b", 7, Unit::Bytes, Escape::Html), "a&amp;b");
        assert_eq!(fit("", 0, Unit::Bytes, Escape::Plain), "");
        assert_eq!(fit("abc", 3, Unit::Utf16, Escape::Plain), "abc");
    }

    #[test]
    fn chinese_is_cut_on_a_character_boundary() {
        let text = "沪深港通资金流入".repeat(10);
        // 3 bytes per character and 3 for the ellipsis: 20 bytes hold five characters.
        let cut = fit(&text, 20, Unit::Bytes, Escape::Plain);
        assert_eq!(cut, "沪深港通资…");
        assert!(cut.len() <= 20);
        // One UTF-16 unit per character: 10 units hold nine characters and the ellipsis.
        assert_eq!(
            fit(&text, 10, Unit::Utf16, Escape::Plain),
            "沪深港通资金流入沪…"
        );
    }

    #[test]
    fn a_cut_never_splits_an_entity() {
        // "a&amp;b" is 7 bytes; with the ellipsis only "a" fits in 6.
        assert_eq!(fit("a&b", 6, Unit::Bytes, Escape::Html), "a…");
        for max in 0..40 {
            let cut = fit("x<y & 风险 > z", max, Unit::Bytes, Escape::Html);
            assert!(cut.len() <= max, "{cut:?} exceeds {max}");
            for (at, _) in cut.match_indices('&') {
                let entity = &cut[at..];
                assert!(
                    ["&amp;", "&lt;", "&gt;"]
                        .iter()
                        .any(|e| entity.starts_with(e)),
                    "broken entity in {cut:?}"
                );
            }
        }
    }

    #[test]
    fn utf16_counts_astral_characters_twice() {
        // Each chart emoji is two UTF-16 units: one fits beside the ellipsis in four.
        assert_eq!(fit("📈📈📈", 4, Unit::Utf16, Escape::Plain), "📈…");
        assert_eq!(fit("📈📈📈", 6, Unit::Utf16, Escape::Plain), "📈📈📈");
    }

    #[test]
    fn no_room_for_the_ellipsis_yields_nothing() {
        assert_eq!(fit("abc", 2, Unit::Bytes, Escape::Plain), "");
        assert_eq!(fit("abc", 0, Unit::Utf16, Escape::Plain), "");
    }
}
