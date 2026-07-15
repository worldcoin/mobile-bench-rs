//! Context-specific Mobench report encoders.

/// Encode untrusted text for an inline Markdown context, including headings.
///
/// The returned text renders as the original plain text while Markdown and
/// HTML parsers see only inert character references. Line breaks are folded
/// to spaces so the value cannot open a new block-level construct.
#[must_use]
pub fn markdown_inline_text(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            encoded.push(' ');
            continue;
        }
        if ch == '\n' {
            encoded.push(' ');
            continue;
        }
        if ch.is_control() {
            encoded.push(' ');
            continue;
        }

        encoded.push_str(match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '!' => "&#33;",
            '"' => "&#34;",
            '#' => "&#35;",
            '(' => "&#40;",
            ')' => "&#41;",
            '*' => "&#42;",
            '+' => "&#43;",
            '-' => "&#45;",
            '.' => "&#46;",
            '/' => "&#47;",
            ':' => "&#58;",
            '@' => "&#64;",
            '[' => "&#91;",
            '\\' => "&#92;",
            ']' => "&#93;",
            '_' => "&#95;",
            '`' => "&#96;",
            '{' => "&#123;",
            '|' => "&#124;",
            '}' => "&#125;",
            '~' => "&#126;",
            _ => {
                encoded.push(ch);
                continue;
            }
        });
    }

    encoded
}

fn markdown_text_requires_encoding(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("mailto:")
        || contains_markdown_underscore_delimiter(input)
        || input.chars().any(|ch| {
            ch.is_control()
                || matches!(
                    ch,
                    '&' | '<'
                        | '>'
                        | '!'
                        | '#'
                        | '('
                        | ')'
                        | '*'
                        | '['
                        | '\\'
                        | ']'
                        | '`'
                        | '{'
                        | '|'
                        | '}'
                        | '~'
                )
        })
}

fn contains_markdown_underscore_delimiter(input: &str) -> bool {
    let chars = input.chars().collect::<Vec<_>>();
    chars.iter().enumerate().any(|(index, ch)| {
        *ch == '_'
            && (index == 0
                || index + 1 == chars.len()
                || !chars[index - 1].is_alphanumeric()
                || !chars[index + 1].is_alphanumeric())
    })
}

/// Encode untrusted text for a GitHub-flavored Markdown table cell.
///
/// This deliberately has a separate interface from [`markdown_inline_text`]
/// so table-specific behavior can evolve without callers choosing a generic
/// escape function. The current encoding is equally strict in both contexts.
#[must_use]
pub fn markdown_table_cell_text(input: &str) -> String {
    markdown_inline_text(input)
}

/// Preserve benign report fields while encoding Markdown-active input.
///
/// This compatibility-oriented wrapper keeps ordinary legacy report values
/// byte-for-byte stable. Values containing Markdown, HTML, control, or
/// autolink syntax delegate to the strict inline encoder.
#[must_use]
pub fn markdown_inline_field_text(input: &str) -> String {
    if markdown_text_requires_encoding(input) {
        markdown_inline_text(input)
    } else {
        input.to_string()
    }
}

/// Wrap untrusted text in a safe inline-code span.
///
/// Ordinary values retain the legacy single-backtick representation. Control
/// characters and line endings are folded to spaces, and values containing
/// backticks use a delimiter longer than every run in the value. Padding keeps
/// the delimiter separate from a leading or trailing backtick; CommonMark
/// removes that padding when the span is rendered.
#[must_use]
pub fn markdown_inline_code(input: &str) -> String {
    let mut normalized = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            normalized.push(' ');
        } else if ch == '\n' || ch.is_control() {
            normalized.push(' ');
        } else {
            normalized.push(ch);
        }
    }

    let max_backtick_run = normalized
        .split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    if max_backtick_run == 0 {
        if normalized.is_empty() {
            return "` `".to_string();
        }
        return format!("`{normalized}`");
    }

    let delimiter = "`".repeat(max_backtick_run + 1);
    format!("{delimiter} {normalized} {delimiter}")
}

/// Preserve benign table fields while encoding Markdown-active input.
///
/// This is the table-context counterpart to [`markdown_inline_field_text`].
#[must_use]
pub fn markdown_table_field_text(input: &str) -> String {
    if markdown_text_requires_encoding(input) {
        markdown_table_cell_text(input)
    } else {
        input.to_string()
    }
}

/// Encode an untrusted relative path for a Markdown link destination.
///
/// RFC 3986 unreserved bytes and non-leading path separators are preserved so
/// normal artifact links stay readable. All other bytes are percent-encoded;
/// leading separators are encoded to prevent protocol-relative destinations.
#[must_use]
pub fn markdown_link_destination(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let bytes = input.as_bytes();
    let leading_slashes = bytes.iter().take_while(|byte| **byte == b'/').count();
    let mut encoded = String::with_capacity(bytes.len());

    for (index, byte) in bytes.iter().copied().enumerate() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        let is_safe_separator = byte == b'/' && index >= leading_slashes;
        if is_unreserved || is_safe_separator {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }

    encoded
}

/// Encode one untrusted CSV field according to RFC 4180.
///
/// Fields containing commas, quotes, CR, or LF are double-quoted and embedded
/// quotes are doubled. To prevent spreadsheet formula execution, an apostrophe
/// is prefixed when the first non-whitespace, non-control character is one of
/// `=`, `+`, `-`, or `@`. Prefixing at byte zero also closes leading
/// whitespace/control-character bypasses while preserving the original text.
#[must_use]
pub fn csv_field(input: &str) -> String {
    let formula_like = input
        .chars()
        .find(|ch| !ch.is_whitespace() && !ch.is_control())
        .is_some_and(|ch| matches!(ch, '=' | '+' | '-' | '@'));

    let neutralized = if formula_like {
        let mut value = String::with_capacity(input.len() + 1);
        value.push('\'');
        value.push_str(input);
        value
    } else {
        input.to_string()
    };

    if !neutralized
        .chars()
        .any(|ch| matches!(ch, ',' | '"' | '\r' | '\n'))
    {
        return neutralized;
    }

    let mut encoded = String::with_capacity(neutralized.len() + 2);
    encoded.push('"');
    for ch in neutralized.chars() {
        if ch == '"' {
            encoded.push('"');
        }
        encoded.push(ch);
    }
    encoded.push('"');
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADVERSARIAL_MARKDOWN_CORPUS: &[&str] = &[
        "# heading\n> quote\n- list item",
        "**bold** _emphasis_ `code` ~~strike~~",
        "[link](https://evil.invalid) ![image](x) <script>alert(1)</script>",
        "left|right\r\nmailto:owner@example.com\\*escaped*",
    ];

    #[test]
    fn inline_text_neutralizes_markdown_html_and_line_breaks() {
        let adversarial =
            "# heading\r\n![image](https://evil.invalid/x) | <script>*owned*</script>";

        let encoded = markdown_inline_text(adversarial);

        assert_eq!(
            encoded,
            "&#35; heading &#33;&#91;image&#93;&#40;https&#58;&#47;&#47;evil&#46;invalid&#47;x&#41; &#124; &lt;script&gt;&#42;owned&#42;&lt;&#47;script&gt;"
        );
    }

    #[test]
    fn table_cell_text_cannot_break_the_row_or_create_links() {
        let adversarial = "left|right\n[link](mailto:owner@example.com) <img src=x>";

        let encoded = markdown_table_cell_text(adversarial);

        assert_eq!(
            encoded,
            "left&#124;right &#91;link&#93;&#40;mailto&#58;owner&#64;example&#46;com&#41; &lt;img src=x&gt;"
        );
    }

    #[test]
    fn both_context_encoders_neutralize_the_adversarial_corpus() {
        for adversarial in ADVERSARIAL_MARKDOWN_CORPUS {
            for encoded in [
                markdown_inline_text(adversarial),
                markdown_table_cell_text(adversarial),
            ] {
                for raw_syntax in [
                    "\r", "\n", "<", ">", "|", "[", "]", "(", ")", "!", "*", "_", "`", "~", "\\",
                    "https://", "mailto:",
                ] {
                    assert!(
                        !encoded.contains(raw_syntax),
                        "encoded output retained `{raw_syntax}` from `{adversarial}`: {encoded}"
                    );
                }
            }
        }
    }

    #[test]
    fn field_encoders_preserve_benign_report_text_exactly() {
        let benign = [
            "2026-03-26T00:00:00Z",
            "Google Pixel 8-14.0",
            "basic_benchmark::bench_fibonacci",
            "plots/alpha-ios.svg",
        ];

        for input in benign {
            assert_eq!(markdown_inline_field_text(input), input);
            assert_eq!(markdown_table_field_text(input), input);
        }
    }

    #[test]
    fn field_encoders_neutralize_underscore_emphasis_without_rewriting_identifiers() {
        assert_eq!(
            markdown_inline_field_text("prefix _emphasis_ suffix"),
            "prefix &#95;emphasis&#95; suffix"
        );
        assert_eq!(
            markdown_table_field_text("__strong__"),
            "&#95;&#95;strong&#95;&#95;"
        );
        assert_eq!(
            markdown_inline_field_text("basic_benchmark::bench_fibonacci"),
            "basic_benchmark::bench_fibonacci"
        );
    }

    #[test]
    fn strict_encoders_remain_canonical_for_benign_report_text() {
        let input = "provekit::passport";

        assert_eq!(markdown_inline_text(input), "provekit&#58;&#58;passport");
        assert_eq!(
            markdown_table_cell_text(input),
            "provekit&#58;&#58;passport"
        );
    }

    #[test]
    fn inline_code_preserves_benign_fields_and_contains_backtick_injection() {
        assert_eq!(
            markdown_inline_code("provekit::passport"),
            "`provekit::passport`"
        );
        assert_eq!(markdown_inline_code("line\r\nbreak"), "`line break`");
        assert_eq!(
            markdown_inline_code("value`\n- forged"),
            "`` value` - forged ``"
        );
        assert_eq!(markdown_inline_code("two``ticks"), "``` two``ticks ```");
        assert_eq!(markdown_inline_code(""), "` `");
    }

    #[test]
    fn csv_field_applies_rfc_4180_quoting_and_formula_neutralization() {
        let cases = [
            ("plain", "plain"),
            ("comma,value", "\"comma,value\""),
            ("quote\"value", "\"quote\"\"value\""),
            ("line\r\nbreak", "\"line\r\nbreak\""),
            ("=SUM(A1:A2)", "'=SUM(A1:A2)"),
            ("+cmd", "'+cmd"),
            ("-1+2", "'-1+2"),
            ("@IMPORT", "'@IMPORT"),
            (" \t=SUM(1,2)", "\"' \t=SUM(1,2)\""),
            ("\u{0000}@cmd", "'\u{0000}@cmd"),
        ];

        for (input, expected) in cases {
            assert_eq!(csv_field(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn csv_field_handles_the_combined_adversarial_corpus() {
        let input = " \t=HYPERLINK(\"https://evil.invalid/a,b\")\r\nnext";

        assert_eq!(
            csv_field(input),
            "\"' \t=HYPERLINK(\"\"https://evil.invalid/a,b\"\")\r\nnext\""
        );
    }

    #[test]
    fn markdown_link_destination_preserves_safe_relative_paths_and_encodes_syntax() {
        let cases = [
            ("plots/alpha.svg", "plots/alpha.svg"),
            ("javascript:alert(1)", "javascript%3Aalert%281%29"),
            ("//evil.invalid/x", "%2F%2Fevil.invalid/x"),
            ("plots/a b[1].svg", "plots/a%20b%5B1%5D.svg"),
            ("plots/é.svg", "plots/%C3%A9.svg"),
            ("plots/x\r\n# heading.svg", "plots/x%0D%0A%23%20heading.svg"),
        ];

        for (input, expected) in cases {
            assert_eq!(
                markdown_link_destination(input),
                expected,
                "input: {input:?}"
            );
        }
    }
}
