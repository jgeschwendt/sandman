//! The memory file itself: ordered frontmatter, blank line, markdown body.
//!
//! The parser is byte-faithful by construction — a field keeps the raw text
//! that followed its colon, so unknown keys, unusual spacing and key order all
//! survive a parse/render round-trip unchanged (`docs/BANK-FORMAT.md`).

use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use crate::error::{Error, Result};

/// The frontmatter fence, with its newline.
const FENCE: &str = "---\n";

/// The four memory types the banks carry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MemoryType {
    /// A correction or stated preference.
    Feedback,
    /// State of a project or system.
    Project,
    /// A durable fact worth looking up.
    Reference,
    /// Something about the person.
    User,
}

impl MemoryType {
    /// The filename prefix and `type:` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
            Self::User => "user",
        }
    }
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "feedback" => Ok(Self::Feedback),
            "project" => Ok(Self::Project),
            "reference" => Ok(Self::Reference),
            "user" => Ok(Self::User),
            other => Err(Error::invalid("memory type", other)),
        }
    }
}

/// One frontmatter line — `key:` plus everything that followed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    key: String,
    /// Verbatim text after the colon, leading space included.
    raw: String,
}

impl Field {
    /// The key, without its colon.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The value, with the single conventional space after the colon removed.
    #[must_use]
    pub fn value(&self) -> &str {
        self.raw.strip_prefix(' ').unwrap_or(&self.raw)
    }
}

/// Ordered `key: value` frontmatter. Order and unknown keys are preserved.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Frontmatter {
    fields: Vec<Field>,
}

impl Frontmatter {
    /// An empty frontmatter block.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The fields, in file order.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// The value of `key`, if present. First occurrence wins.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .map(Field::value)
    }

    /// Set `key`, keeping its position if it already exists and appending
    /// otherwise.
    pub fn set(&mut self, key: &str, value: impl AsRef<str>) {
        let raw = format!(" {}", value.as_ref());
        if let Some(field) = self.fields.iter_mut().find(|field| field.key == key) {
            field.raw = raw;
        } else {
            self.fields.push(Field {
                key: key.to_owned(),
                raw,
            });
        }
    }

    /// Remove `key`, returning its value if it was there.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        let index = self.fields.iter().position(|field| field.key == key)?;
        Some(self.fields.remove(index).value().to_owned())
    }
}

/// A parsed memory file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryFile {
    /// The frontmatter block, in file order.
    pub frontmatter: Frontmatter,
    /// Everything after the blank line that follows the closing fence.
    pub body: String,
}

impl MemoryFile {
    /// Build a file from frontmatter and a body.
    #[must_use]
    pub fn new(frontmatter: Frontmatter, body: impl Into<String>) -> Self {
        Self {
            frontmatter,
            body: body.into(),
        }
    }

    /// Parse a memory file. Byte-exact: [`MemoryFile::render`] reproduces the
    /// input for every well-formed file.
    pub fn parse(text: &str) -> std::result::Result<Self, ParseError> {
        let mut cursor = text
            .strip_prefix(FENCE)
            .ok_or_else(|| ParseError::new(1, ParseErrorKind::MissingFrontmatter))?;
        let mut fields = Vec::new();
        // Line 1 is the opening fence.
        let mut line_no = 1_usize;
        loop {
            line_no += 1;
            let (line, tail) = split_line(cursor);
            let Some(tail) = tail else {
                return Err(ParseError::new(
                    line_no,
                    ParseErrorKind::UnterminatedFrontmatter,
                ));
            };
            if line == "---" {
                let body = tail.strip_prefix('\n').ok_or_else(|| {
                    ParseError::new(line_no + 1, ParseErrorKind::MissingBlankLine)
                })?;
                return Ok(Self {
                    frontmatter: Frontmatter { fields },
                    body: body.to_owned(),
                });
            }
            let colon = line
                .find(':')
                .ok_or_else(|| ParseError::new(line_no, ParseErrorKind::MalformedField))?;
            if colon == 0 {
                return Err(ParseError::new(line_no, ParseErrorKind::EmptyKey));
            }
            fields.push(Field {
                key: line[..colon].to_owned(),
                raw: line[colon + 1..].to_owned(),
            });
            cursor = tail;
        }
    }

    /// Read and parse a memory file from disk.
    pub fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
        Self::parse(&text).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Serialize back to the on-disk form.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(self.body.len() + 128);
        out.push_str(FENCE);
        for field in &self.frontmatter.fields {
            out.push_str(&field.key);
            out.push(':');
            out.push_str(&field.raw);
            out.push('\n');
        }
        out.push_str(FENCE);
        out.push('\n');
        out.push_str(&self.body);
        out
    }

    /// The `name:` value, if present.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.frontmatter.get("name")
    }

    /// The `description:` value, if present.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.frontmatter.get("description")
    }
}

/// Split at the first newline. `None` tail means the line was unterminated.
fn split_line(s: &str) -> (&str, Option<&str>) {
    match s.find('\n') {
        Some(index) => (&s[..index], Some(&s[index + 1..])),
        None => (s, None),
    }
}

/// Why a memory file failed to parse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseErrorKind {
    /// A frontmatter line had a colon in column zero.
    EmptyKey,
    /// A frontmatter line carried no colon.
    MalformedField,
    /// No blank line between the closing fence and the body.
    MissingBlankLine,
    /// The file does not open with `---`.
    MissingFrontmatter,
    /// The frontmatter block is never closed.
    UnterminatedFrontmatter,
}

impl ParseErrorKind {
    /// A one-line explanation.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::EmptyKey => "frontmatter key is empty",
            Self::MalformedField => "frontmatter line is not `key: value`",
            Self::MissingBlankLine => "no blank line between frontmatter and body",
            Self::MissingFrontmatter => "file does not start with `---`",
            Self::UnterminatedFrontmatter => "frontmatter is never closed with `---`",
        }
    }
}

/// A parse failure, located by line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseError {
    /// 1-based line number the failure was detected at.
    pub line: usize,
    /// What went wrong.
    pub kind: ParseErrorKind,
}

impl ParseError {
    /// Build a located parse error.
    #[must_use]
    pub const fn new(line: usize, kind: ParseErrorKind) -> Self {
        Self { line, kind }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.kind.message())
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::{MemoryFile, MemoryType, ParseErrorKind};

    fn round_trip(text: &str) {
        let parsed = MemoryFile::parse(text).expect("well-formed");
        assert_eq!(parsed.render(), text);
    }

    #[test]
    fn round_trips_the_dominant_shape() {
        round_trip(concat!(
            "---\n",
            "name: orrery-extraction\n",
            "description: one line\n",
            "type: project\n",
            "created: 2026-07-20T02:43:00Z\n",
            "source: 2026-07-19 orrery extraction session\n",
            "updated: 2026-07-20T02:43:00Z\n",
            "---\n",
            "\n",
            "Body text.\n",
        ));
    }

    #[test]
    fn round_trips_an_unknown_key_in_place() {
        let text = concat!(
            "---\n",
            "name: orrery-extraction\n",
            "description: one line\n",
            "type: project\n",
            "created: 2026-07-20T02:43:00Z\n",
            "recall: index\n",
            "source: session\n",
            "updated: 2026-07-20T02:43:00Z\n",
            "---\n",
            "\n",
            "Body.\n",
        );
        round_trip(text);
        let parsed = MemoryFile::parse(text).expect("well-formed");
        let keys: Vec<&str> = parsed
            .frontmatter
            .fields()
            .iter()
            .map(super::Field::key)
            .collect();
        assert_eq!(
            keys,
            [
                "name",
                "description",
                "type",
                "created",
                "recall",
                "source",
                "updated"
            ]
        );
        assert_eq!(parsed.frontmatter.get("recall"), Some("index"));
    }

    #[test]
    fn round_trips_without_created_updated_or_source() {
        round_trip(concat!(
            "---\n",
            "name: n\n",
            "description: d\n",
            "type: user\n",
            "---\n",
            "\n",
            "Body.\n",
        ));
    }

    #[test]
    fn round_trips_unicode_in_values_and_body() {
        let text = concat!(
            "---\n",
            "name: game_01: Zelda/LA-remake diorama game, ported web→Rust+Bevy\n",
            "description: em—dashes — and · middots, “quotes”, 🌙\n",
            "type: project\n",
            "---\n",
            "\n",
            "Body with — an em dash, a → arrow and a 🌙 emoji.\n",
            "\n",
            "## A heading\n",
        );
        round_trip(text);
        let parsed = MemoryFile::parse(text).expect("well-formed");
        assert_eq!(
            parsed.name(),
            Some("game_01: Zelda/LA-remake diorama game, ported web→Rust+Bevy")
        );
    }

    #[test]
    fn round_trips_odd_spacing_and_empty_values() {
        round_trip(concat!(
            "---\n",
            "name:\n",
            "description:  two leading spaces\n",
            "type: user\n",
            "trailing: value with trailing space \n",
            "---\n",
            "\n",
            "Body.\n",
        ));
    }

    #[test]
    fn round_trips_a_body_containing_a_fence() {
        round_trip(concat!(
            "---\n",
            "name: n\n",
            "description: d\n",
            "type: user\n",
            "---\n",
            "\n",
            "Body.\n",
            "\n",
            "---\n",
            "\n",
            "More body.\n",
        ));
    }

    #[test]
    fn round_trips_an_empty_body() {
        round_trip("---\nname: n\ndescription: d\ntype: user\n---\n\n");
    }

    #[test]
    fn parse_failures_are_typed() {
        let cases = [
            ("no fence\n", 1, ParseErrorKind::MissingFrontmatter),
            ("---\nname: n\n", 3, ParseErrorKind::UnterminatedFrontmatter),
            ("---\nname\n---\n\n", 2, ParseErrorKind::MalformedField),
            ("---\n: v\n---\n\n", 2, ParseErrorKind::EmptyKey),
            (
                "---\nname: n\n---\nbody\n",
                4,
                ParseErrorKind::MissingBlankLine,
            ),
            (
                "---\nname: n\n---",
                3,
                ParseErrorKind::UnterminatedFrontmatter,
            ),
        ];
        for (text, line, kind) in cases {
            let err = MemoryFile::parse(text).expect_err("must fail");
            assert_eq!((err.line, err.kind), (line, kind), "for {text:?}");
        }
    }

    #[test]
    fn set_keeps_position_and_remove_drops_it() {
        let mut parsed =
            MemoryFile::parse("---\nname: n\ndescription: d\ntype: user\n---\n\nb\n").expect("ok");
        parsed.frontmatter.set("description", "changed");
        parsed.frontmatter.set("updated", "2026-08-11T00:00:00Z");
        assert_eq!(
            parsed.render(),
            "---\nname: n\ndescription: changed\ntype: user\nupdated: 2026-08-11T00:00:00Z\n---\n\nb\n"
        );
        assert_eq!(parsed.frontmatter.remove("type").as_deref(), Some("user"));
        assert_eq!(parsed.frontmatter.get("type"), None);
    }

    #[test]
    fn memory_types_round_trip_through_strings() {
        for kind in [
            MemoryType::Feedback,
            MemoryType::Project,
            MemoryType::Reference,
            MemoryType::User,
        ] {
            assert_eq!(kind.as_str().parse::<MemoryType>().expect("known"), kind);
        }
        assert!("lore".parse::<MemoryType>().is_err());
    }
}
