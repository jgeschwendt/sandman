//! TOML for our own shape only — the ruach's `breath/sources.toml`.
//!
//! The subset is flat: root keys, then `[name]` tables of `key = value` lines,
//! where a value is a basic or literal string, an integer, a float, or a
//! one-line array of strings. Comments run from `#` to the end of a line.
//! Anything else — a dotted or nested table, an array of tables, an inline
//! table, a boolean, a date, a multi-line string or array, a duplicate key —
//! is refused with its line number rather than half-read. Like `json.rs`, this
//! is why `[dependencies]` stays empty.
//!
//! [`Document::render`] writes the subset back deterministically: root keys,
//! then tables, each in the order held, values canonical. Comments are not
//! kept.

use std::fmt::{self, Write as _};

/// One value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A float — always rendered with a decimal point.
    Float(f64),
    /// A signed integer.
    Integer(i64),
    /// A string.
    String(String),
    /// A one-line array of strings.
    Strings(Vec<String>),
}

impl Value {
    /// The string, when this is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// The number, an integer widened.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(number) => Some(*number),
            #[allow(
                clippy::cast_precision_loss,
                reason = "weights and budgets are small; exactness past 2^52 is moot"
            )]
            Self::Integer(number) => Some(*number as f64),
            _ => None,
        }
    }

    /// The integer, when this is one.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(number) => Some(*number),
            _ => None,
        }
    }

    /// The strings, when this is an array of them.
    #[must_use]
    pub fn as_strings(&self) -> Option<&[String]> {
        match self {
            Self::Strings(items) => Some(items),
            _ => None,
        }
    }

    /// The canonical text.
    fn render(&self) -> String {
        match self {
            Self::Float(number) => render_float(*number),
            Self::Integer(number) => number.to_string(),
            Self::String(text) => quote(text),
            Self::Strings(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|item| quote(item))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// `[name]` and its keys, in the order held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Its keys and values.
    pub entries: Vec<(String, Value)>,
    /// The bare key in the brackets.
    pub name: String,
}

impl Table {
    /// The value under `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }
}

/// A whole file: the keys before the first table, then the tables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    /// Keys before any `[table]`.
    pub root: Vec<(String, Value)>,
    /// The tables, in file order.
    pub tables: Vec<Table>,
}

impl Document {
    /// The subset rendered back: root keys, a blank line, then each table
    /// separated by one blank line.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (key, value) in &self.root {
            let _ = writeln!(out, "{key} = {}", value.render());
        }
        for table in &self.tables {
            if !out.is_empty() {
                out.push('\n');
            }
            let _ = writeln!(out, "[{}]", table.name);
            for (key, value) in &table.entries {
                let _ = writeln!(out, "{key} = {}", value.render());
            }
        }
        out
    }
}

/// Where and why a file left the subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// One-based.
    pub line: usize,
    /// What was wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Read the subset.
pub fn parse(text: &str) -> Result<Document, ParseError> {
    let mut document = Document::default();
    let mut names: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |message: String| ParseError { line, message };
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("[[") {
            return Err(fail("arrays of tables are not supported".to_owned()));
        }
        if let Some(rest) = trimmed.strip_prefix('[') {
            let close = rest
                .find(']')
                .ok_or_else(|| fail("a table header wants a closing `]`".to_owned()))?;
            let name = rest[..close].trim();
            let after = rest[close + 1..].trim();
            if !after.is_empty() && !after.starts_with('#') {
                return Err(fail(format!("unexpected `{after}` after the table header")));
            }
            if name.contains('.') {
                return Err(fail(format!("nested table `{name}` is not supported")));
            }
            if !is_bare(name) {
                return Err(fail(format!("table name `{name}` is not a bare key")));
            }
            if names.iter().any(|seen| seen == name) {
                return Err(fail(format!("table `{name}` is defined twice")));
            }
            names.push(name.to_owned());
            document.tables.push(Table {
                entries: Vec::new(),
                name: name.to_owned(),
            });
            continue;
        }
        let equals = trimmed
            .find('=')
            .ok_or_else(|| fail(format!("expected `key = value`, got `{trimmed}`")))?;
        let key = trimmed[..equals].trim();
        if key.contains('.') {
            return Err(fail(format!("dotted key `{key}` is not supported")));
        }
        if !is_bare(key) {
            return Err(fail(format!("key `{key}` is not a bare key")));
        }
        let (value, rest) = value(trimmed[equals + 1..].trim_start()).map_err(fail)?;
        let rest = rest.trim();
        if !rest.is_empty() && !rest.starts_with('#') {
            return Err(fail(format!("unexpected `{rest}` after the value")));
        }
        let entries = match document.tables.last_mut() {
            Some(table) => &mut table.entries,
            None => &mut document.root,
        };
        if entries.iter().any(|(seen, _)| seen == key) {
            return Err(fail(format!("key `{key}` is defined twice")));
        }
        entries.push((key.to_owned(), value));
    }
    Ok(document)
}

/// A bare key: ASCII letters, digits, `_` and `-`, at least one.
fn is_bare(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// One value off the front of `text`, and what follows it.
fn value(text: &str) -> Result<(Value, &str), String> {
    match text.chars().next() {
        None => Err("a key wants a value".to_owned()),
        Some('"' | '\'') => {
            if text.starts_with("\"\"\"") || text.starts_with("'''") {
                return Err("multi-line strings are not supported".to_owned());
            }
            let (text, rest) = string(text)?;
            Ok((Value::String(text), rest))
        }
        Some('[') => strings(text),
        Some('{') => Err("inline tables are not supported".to_owned()),
        Some(_) => {
            let end = text.find(['#', ' ', '\t']).unwrap_or(text.len());
            Ok((number(&text[..end])?, &text[end..]))
        }
    }
}

/// A one-line array of strings.
fn strings(text: &str) -> Result<(Value, &str), String> {
    let mut rest = text[1..].trim_start();
    let mut items = Vec::new();
    loop {
        if let Some(after) = rest.strip_prefix(']') {
            return Ok((Value::Strings(items), after));
        }
        if rest.is_empty() || rest.starts_with('#') {
            return Err("arrays must close on the line they open".to_owned());
        }
        if !rest.starts_with(['"', '\'']) {
            return Err("arrays hold strings only".to_owned());
        }
        let (item, after) = string(rest)?;
        items.push(item);
        rest = after.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
        } else if !rest.starts_with(']') {
            return Err("array items want a `,` between them".to_owned());
        }
    }
}

/// A basic (`"…"`, with escapes) or literal (`'…'`) string.
fn string(text: &str) -> Result<(String, &str), String> {
    let quote = text.chars().next().unwrap_or('"');
    let mut out = String::new();
    let mut chars = text[1..].char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            _ if ch == quote => return Ok((out, &text[1 + index + 1..])),
            '\\' if quote == '"' => {
                let (_, escaped) = chars
                    .next()
                    .ok_or_else(|| "a string ends inside an escape".to_owned())?;
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = (0..4)
                            .filter_map(|_| chars.next())
                            .map(|(_, c)| c)
                            .collect();
                        let code = u32::from_str_radix(&hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .filter(|_| hex.len() == 4)
                            .ok_or_else(|| format!("bad unicode escape `\\u{hex}`"))?;
                        out.push(code);
                    }
                    other => return Err(format!("unsupported escape `\\{other}`")),
                }
            }
            _ => out.push(ch),
        }
    }
    Err("a string wants its closing quote on the same line".to_owned())
}

/// An integer or a float; booleans and dates are refused by name.
fn number(text: &str) -> Result<Value, String> {
    if matches!(text, "true" | "false") {
        return Err("booleans are not supported".to_owned());
    }
    let digits = text.trim_start_matches(['+', '-']);
    let plain = !digits.is_empty()
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
        && digits.as_bytes()[0].is_ascii_digit();
    if !plain {
        return Err(format!("unsupported value `{text}`"));
    }
    if let Ok(integer) = text.parse::<i64>() {
        return Ok(Value::Integer(integer));
    }
    if digits.contains(['.', 'e', 'E'])
        && let Ok(float) = text.parse::<f64>()
        && float.is_finite()
    {
        return Ok(Value::Float(float));
    }
    Err(format!("unsupported value `{text}`"))
}

/// A float with a decimal point always — `1.0`, never `1`.
fn render_float(number: f64) -> String {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E', 'i', 'N']) {
        text
    } else {
        format!("{text}.0")
    }
}

/// A basic string, escaped.
fn quote(text: &str) -> String {
    let mut out = String::from('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(ch));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{Document, Table, Value, parse};

    #[test]
    fn the_flat_shape_reads_every_value_kind() {
        let doc = parse(
            "top = \"x\" # trailing\n\n[a-b]\ns = \"q\\\"\\u00e9\\n\"\nl = 'raw\\'\ni = -3\nf = 1.5\ne = 2e3\narr = [\"*\", 'x' , \"y\",]\nnone = []\n",
        )
        .expect("parse");
        assert_eq!(
            doc.root,
            [("top".to_owned(), Value::String("x".to_owned()))]
        );
        let table = &doc.tables[0];
        assert_eq!(table.name, "a-b");
        assert_eq!(table.get("s"), Some(&Value::String("q\"é\n".to_owned())));
        assert_eq!(table.get("l"), Some(&Value::String("raw\\".to_owned())));
        assert_eq!(table.get("i"), Some(&Value::Integer(-3)));
        assert_eq!(table.get("f"), Some(&Value::Float(1.5)));
        assert_eq!(table.get("e"), Some(&Value::Float(2000.0)));
        assert_eq!(
            table.get("arr").and_then(Value::as_strings),
            Some(&["*".to_owned(), "x".to_owned(), "y".to_owned()][..])
        );
        assert_eq!(table.get("none"), Some(&Value::Strings(Vec::new())));
    }

    #[test]
    fn anything_outside_the_subset_is_refused_with_its_line() {
        for (text, line, needle) in [
            ("[a.b]\n", 1, "nested table"),
            ("[[a]]\n", 1, "arrays of tables"),
            ("[a]\nx = { y = 1 }\n", 2, "inline tables"),
            ("[a]\nx = true\n", 2, "booleans"),
            ("[a]\nx = 2026-10-09\n", 2, "unsupported value"),
            ("[a]\nx = [1, 2]\n", 2, "strings only"),
            ("[a]\nx = [\"a\",\n\"b\"]\n", 2, "close on the line"),
            ("[a]\nx = \"\"\"long\"\"\"\n", 2, "multi-line strings"),
            ("[a]\nx = \"open\n", 2, "closing quote"),
            ("[a]\nx = 1\nx = 2\n", 3, "defined twice"),
            ("[a]\n[a]\n", 2, "defined twice"),
            ("[a]\na.b = 1\n", 2, "dotted key"),
            ("[a]\njust words\n", 2, "key = value"),
            ("[a]\nx = 1 2\n", 2, "after the value"),
            ("[a]\nx = \"\\q\"\n", 2, "unsupported escape"),
            ("[a]\nx = inf\n", 2, "unsupported value"),
        ] {
            let error = parse(text).expect_err(text);
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert!(error.message.contains(needle), "{text:?}: {error}");
        }
    }

    #[test]
    fn render_is_canonical_and_reads_back() {
        let doc = Document {
            root: vec![("at".to_owned(), Value::String("t".to_owned()))],
            tables: vec![Table {
                entries: vec![
                    ("w".to_owned(), Value::Float(1.0)),
                    ("b".to_owned(), Value::Integer(10)),
                    ("o".to_owned(), Value::Strings(vec!["a\"b".to_owned()])),
                    ("c".to_owned(), Value::String("tab\there".to_owned())),
                ],
                name: "src".to_owned(),
            }],
        };
        let text = doc.render();
        assert_eq!(
            text,
            "at = \"t\"\n\n[src]\nw = 1.0\nb = 10\no = [\"a\\\"b\"]\nc = \"tab\\there\"\n"
        );
        assert_eq!(parse(&text).expect("reparse"), doc);
    }
}
