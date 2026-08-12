//! A tiny JSON reader and writer — sandman's own shapes only.
//!
//! Pointer files and hook payloads are the whole surface; this is not a
//! general-purpose library, it is the reason `[dependencies]` stays empty.
//! Strings escape and unescape exactly (`\"`, `\\`, `\n`, `\r`, `\t`,
//! `\uXXXX`), objects keep their key order, and every failure is typed so a
//! caller can drop one malformed transcript line without losing the file.

use std::fmt::{self, Write as _};

/// How deep a value may nest before the parser gives up. Transcript lines
/// carry model payloads of unknown shape; a bound turns a hostile file into a
/// skipped line instead of a blown stack.
const MAX_DEPTH: usize = 256;

/// The replacement character, emitted for an unpaired surrogate escape.
const REPLACEMENT: char = '\u{fffd}';

/// A JSON value. Objects keep their keys in file order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// `[…]`
    Array(Vec<Value>),
    /// `true` / `false`
    Bool(bool),
    /// `null`
    Null,
    /// Any number, held as `f64`.
    Number(f64),
    /// `{…}`, in file order.
    Object(Vec<(String, Value)>),
    /// A string, unescaped.
    String(String),
}

impl Value {
    /// The value at `key`, when this is an object carrying one.
    pub(crate) fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .find(|(entry, _)| entry == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The flag, when this is a boolean.
    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The number, when this is one.
    pub(crate) fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// The elements, when this is an array.
    pub(crate) fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The text, when this is a string.
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// A string value, built from anything string-like.
    pub(crate) fn string(text: impl Into<String>) -> Self {
        Self::String(text.into())
    }

    /// The on-the-wire form — compact, no spaces.
    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        self.render_into(&mut out);
        out
    }

    /// Append the on-the-wire form to `out`.
    fn render_into(&self, out: &mut String) {
        match self {
            Self::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.render_into(out);
                }
                out.push(']');
            }
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::Null => out.push_str("null"),
            Self::Number(value) => out.push_str(&render_number(*value)),
            Self::Object(entries) => {
                out.push('{');
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    escape_into(out, key);
                    out.push(':');
                    value.render_into(out);
                }
                out.push('}');
            }
            Self::String(text) => escape_into(out, text),
        }
    }
}

/// Numbers render as integers when they are integral; JSON has no `NaN` or
/// infinity, so both become `null`.
fn render_number(value: f64) -> String {
    if !value.is_finite() {
        return "null".to_owned();
    }
    if value.fract().abs() < f64::EPSILON && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// Append `text` to `out` as a quoted JSON string.
pub(crate) fn escape_into(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                // Infallible: writing into a String never fails.
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Parse one complete JSON value. Trailing content is an error.
pub(crate) fn parse(text: &str) -> Result<Value, JsonError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.pos == parser.bytes.len() {
        Ok(value)
    } else {
        Err(JsonError::Trailing(parser.pos))
    }
}

/// Why a parse failed. Offsets are byte positions into the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsonError {
    /// Nesting ran past [`MAX_DEPTH`].
    Depth(usize),
    /// The input ended mid-value.
    Eof,
    /// A `\` escape sandman does not accept.
    Escape(usize),
    /// A number that `f64` cannot read.
    Number(usize),
    /// Content after the top-level value.
    Trailing(usize),
    /// A byte that cannot start what the grammar expects here.
    Unexpected(usize),
    /// A string's bytes are not UTF-8.
    Utf8(usize),
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Depth(at) => write!(f, "json nested past {MAX_DEPTH} levels at byte {at}"),
            Self::Eof => write!(f, "json ended mid-value"),
            Self::Escape(at) => write!(f, "unsupported json escape at byte {at}"),
            Self::Number(at) => write!(f, "malformed json number at byte {at}"),
            Self::Trailing(at) => write!(f, "trailing json content at byte {at}"),
            Self::Unexpected(at) => write!(f, "unexpected json byte at {at}"),
            Self::Utf8(at) => write!(f, "json string is not utf-8, ending at byte {at}"),
        }
    }
}

impl std::error::Error for JsonError {}

/// A cursor over the input bytes.
struct Parser<'a> {
    /// The whole input.
    bytes: &'a [u8],
    /// How far the cursor has read.
    pos: usize,
}

impl Parser<'_> {
    /// The byte under the cursor.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Step over insignificant whitespace.
    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// The error for the byte under the cursor.
    fn unexpected(&self) -> JsonError {
        if self.pos >= self.bytes.len() {
            JsonError::Eof
        } else {
            JsonError::Unexpected(self.pos)
        }
    }

    /// Consume `byte`, or fail.
    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    /// One value at `depth`.
    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::Depth(self.pos));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.unexpected()),
        }
    }

    /// One of the three bare words.
    fn literal(&mut self, word: &str, value: Value) -> Result<Value, JsonError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.unexpected())
        }
    }

    /// `{ "key": value, … }`.
    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(entries));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    /// `[ value, … ]`.
    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    /// A number, read as `f64`.
    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| JsonError::Number(start))?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| JsonError::Number(start))
    }

    /// A quoted string, unescaped.
    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let byte = self.peek().ok_or(JsonError::Eof)?;
            self.pos += 1;
            match byte {
                b'"' => return String::from_utf8(out).map_err(|_| JsonError::Utf8(self.pos)),
                b'\\' => self.escape(&mut out)?,
                // Raw control bytes are not legal inside a JSON string.
                0x00..=0x1f => return Err(JsonError::Unexpected(self.pos - 1)),
                other => out.push(other),
            }
        }
    }

    /// The character after a `\`.
    fn escape(&mut self, out: &mut Vec<u8>) -> Result<(), JsonError> {
        let byte = self.peek().ok_or(JsonError::Eof)?;
        self.pos += 1;
        let ch = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode_escape(out),
            _ => return Err(JsonError::Escape(self.pos - 1)),
        };
        push_char(out, ch);
        Ok(())
    }

    /// `\uXXXX`, surrogate pairs included. A lone surrogate degrades to
    /// U+FFFD rather than failing: one bad escape must not cost the line.
    fn unicode_escape(&mut self, out: &mut Vec<u8>) -> Result<(), JsonError> {
        let first = self.hex4()?;
        let code = if (0xd800..0xdc00).contains(&first) {
            match self.paired_low_surrogate()? {
                Some(low) => 0x10000 + ((first - 0xd800) << 10) + (low - 0xdc00),
                None => REPLACEMENT as u32,
            }
        } else if (0xdc00..0xe000).contains(&first) {
            REPLACEMENT as u32
        } else {
            first
        };
        push_char(out, char::from_u32(code).unwrap_or(REPLACEMENT));
        Ok(())
    }

    /// The `\uXXXX` following a high surrogate, when it is a low one.
    fn paired_low_surrogate(&mut self) -> Result<Option<u32>, JsonError> {
        if !self.bytes[self.pos..].starts_with(b"\\u") {
            return Ok(None);
        }
        let mark = self.pos;
        self.pos += 2;
        let low = self.hex4()?;
        if (0xdc00..0xe000).contains(&low) {
            return Ok(Some(low));
        }
        self.pos = mark;
        Ok(None)
    }

    /// Four hex digits — and only hex digits, so no sign slips through.
    fn hex4(&mut self) -> Result<u32, JsonError> {
        let start = self.pos;
        let digits = self
            .bytes
            .get(start..start + 4)
            .ok_or(JsonError::Eof)?
            .to_owned();
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err(JsonError::Escape(start));
        }
        let text = std::str::from_utf8(&digits).map_err(|_| JsonError::Escape(start))?;
        let value = u32::from_str_radix(text, 16).map_err(|_| JsonError::Escape(start))?;
        self.pos += 4;
        Ok(value)
    }
}

/// Append one character's UTF-8 bytes.
fn push_char(out: &mut Vec<u8>, ch: char) {
    let mut buffer = [0_u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::{JsonError, Value, escape_into, parse};

    fn quoted(text: &str) -> String {
        let mut out = String::new();
        escape_into(&mut out, text);
        out
    }

    fn round_trip(text: &str) {
        let encoded = quoted(text);
        match parse(&encoded) {
            Ok(Value::String(decoded)) => assert_eq!(decoded, text, "via {encoded}"),
            other => panic!("{encoded} parsed as {other:?}"),
        }
    }

    #[test]
    fn strings_round_trip_through_escaping() {
        for text in [
            "",
            "plain",
            "quote \" backslash \\ slash /",
            "newline \n tab \t carriage \r",
            "control \u{1} \u{1f} bell \u{7}",
            "unicode — · “quotes” 🌙 é",
            "mixed \"body\" with 🌙 and \n",
        ] {
            round_trip(text);
        }
    }

    #[test]
    fn escaping_uses_the_documented_forms() {
        assert_eq!(quoted("a\"b"), r#""a\"b""#);
        assert_eq!(quoted("a\\b"), r#""a\\b""#);
        assert_eq!(quoted("a\nb\tc\rd"), r#""a\nb\tc\rd""#);
        assert_eq!(quoted("\u{0}\u{1f}"), "\"\\u0000\\u001f\"");
        // Non-ASCII stays literal — the transcript files are UTF-8.
        assert_eq!(quoted("🌙"), "\"🌙\"");
    }

    #[test]
    fn unescaping_reads_every_escape_form() {
        let text = r#""\" \\ \/ \b \f \n \r \t A é 🌙""#;
        let Ok(Value::String(decoded)) = parse(text) else {
            panic!("parse failed");
        };
        assert_eq!(decoded, "\" \\ / \u{8} \u{c} \n \r \t A é 🌙");
    }

    #[test]
    fn a_lone_surrogate_degrades_to_the_replacement_character() {
        let Ok(Value::String(decoded)) = parse(r#""\ud83c!""#) else {
            panic!("parse failed");
        };
        assert_eq!(decoded, "\u{fffd}!");
        let Ok(Value::String(decoded)) = parse(r#""\udf19""#) else {
            panic!("parse failed");
        };
        assert_eq!(decoded, "\u{fffd}");
    }

    #[test]
    fn objects_keep_key_order_and_render_compactly() {
        let value = Value::Object(vec![
            ("zulu".to_owned(), Value::string("last")),
            ("alpha".to_owned(), Value::Array(vec![Value::Number(1.0)])),
            ("nested".to_owned(), Value::Object(vec![])),
            ("flag".to_owned(), Value::Bool(false)),
            ("nothing".to_owned(), Value::Null),
        ]);
        assert_eq!(
            value.render(),
            r#"{"zulu":"last","alpha":[1],"nested":{},"flag":false,"nothing":null}"#
        );
        assert_eq!(parse(&value.render()).expect("re-parse"), value);
    }

    #[test]
    fn numbers_survive_a_round_trip() {
        for (text, expected) in [
            ("0", 0.0),
            ("-17", -17.0),
            ("1.5", 1.5),
            ("1e3", 1000.0),
            ("-2.5e-2", -0.025),
        ] {
            let Ok(Value::Number(value)) = parse(text) else {
                panic!("{text} did not parse as a number");
            };
            assert!((value - expected).abs() < 1e-12, "{text} → {value}");
        }
        assert_eq!(Value::Number(3.0).render(), "3");
        assert_eq!(Value::Number(1.5).render(), "1.5");
        assert_eq!(Value::Number(f64::NAN).render(), "null");
    }

    #[test]
    fn accessors_reach_into_our_own_shapes() {
        let value =
            parse(r#"{"session_id":"abc","items":[{"type":"text"}],"n":2}"#).expect("parse");
        assert_eq!(value.get("session_id").and_then(Value::as_str), Some("abc"));
        assert_eq!(
            value
                .get("items")
                .and_then(Value::as_array)
                .and_then(<[Value]>::first)
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("text")
        );
        assert_eq!(value.get("missing"), None);
        assert_eq!(value.as_str(), None);

        let flags = parse(r#"{"is_error":false,"n":2.5,"text":"x"}"#).expect("parse");
        assert_eq!(flags.get("is_error").and_then(Value::as_bool), Some(false));
        assert_eq!(flags.get("n").and_then(Value::as_number), Some(2.5));
        assert_eq!(flags.get("text").and_then(Value::as_bool), None);
        assert_eq!(flags.get("text").and_then(Value::as_number), None);
    }

    #[test]
    fn malformed_input_is_a_typed_error() {
        assert_eq!(parse(""), Err(JsonError::Eof));
        assert_eq!(parse("{"), Err(JsonError::Eof));
        assert!(matches!(parse("{}{}"), Err(JsonError::Trailing(2))));
        assert!(matches!(parse("{\"a\" 1}"), Err(JsonError::Unexpected(_))));
        assert!(matches!(parse(r#""\q""#), Err(JsonError::Escape(_))));
        assert!(matches!(parse(r#""\u00g0""#), Err(JsonError::Escape(_))));
        assert!(matches!(parse("nul"), Err(JsonError::Unexpected(_))));
        assert!(matches!(parse("[1,]"), Err(JsonError::Unexpected(_))));
        assert!(matches!(
            parse("\"raw\nnewline\""),
            Err(JsonError::Unexpected(_))
        ));
    }

    #[test]
    fn nesting_past_the_bound_is_refused_not_a_crash() {
        let deep = format!("{}{}", "[".repeat(1_000), "]".repeat(1_000));
        assert!(matches!(parse(&deep), Err(JsonError::Depth(_))));
    }
}
