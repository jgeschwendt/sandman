//! Typed errors for every fallible path in the crate.
//!
//! The commit path never panics: a malformed file, a held lock or a hostile
//! filename all surface as an [`Error`] variant.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::memory::ParseError;

/// Result alias carrying [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong under the commit path.
#[derive(Debug)]
pub enum Error {
    /// The system clock reports a time before the Unix epoch.
    Clock,
    /// A rename would have crossed a filesystem boundary. Archiving is a move
    /// or it is nothing — sandman never copies a transcript.
    CrossDevice {
        /// What was being moved.
        from: PathBuf,
        /// Where it was headed.
        to: PathBuf,
    },
    /// A filesystem operation failed on `path`.
    Io {
        /// The path the operation was attempted on.
        path: PathBuf,
        /// The underlying OS error.
        source: io::Error,
    },
    /// A caller-supplied value is not usable — empty, multi-line, or a path
    /// component that would escape the bank.
    InvalidInput {
        /// Which input was rejected.
        what: &'static str,
        /// The offending value.
        value: String,
    },
    /// A JSON payload — a hook's stdin, a pointer file — is not well-formed.
    Json {
        /// The file it came from, when it came from one.
        path: Option<PathBuf>,
        /// What the JSON reader said.
        message: String,
    },
    /// The data root's `.commit.lock` is already held by another writer.
    LockHeld {
        /// The lockfile that could not be created.
        path: PathBuf,
    },
    /// A required environment variable is not set.
    MissingEnv {
        /// The variable's name.
        name: &'static str,
    },
    /// A memory file lacks a frontmatter key the caller needs.
    MissingField {
        /// The missing key.
        key: &'static str,
        /// The file that lacks it.
        path: PathBuf,
    },
    /// Something the caller named is not there.
    NotFound {
        /// What kind of thing was looked for.
        what: &'static str,
        /// The name that found nothing.
        value: String,
    },
    /// A memory file is not well-formed.
    Parse {
        /// The file that failed to parse.
        path: PathBuf,
        /// Where and how it failed.
        source: ParseError,
    },
    /// A deliberate refusal: a live session, a destroy with nothing to
    /// destroy. Not a failure — a decision.
    Refused {
        /// The one line the operator gets.
        message: String,
    },
    /// `replaces` named a file that is not in the bank.
    ReplacesMissing {
        /// The path the replaced file was expected at.
        path: PathBuf,
    },
    /// Collision-suffix resolution ran past its bound.
    TooManyCollisions {
        /// The base filename that could not be placed.
        filename: String,
    },
}

impl Error {
    /// Attach a path to an [`io::Error`].
    pub(crate) fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    /// Reject a caller-supplied value.
    pub(crate) fn invalid(what: &'static str, value: impl Into<String>) -> Self {
        Self::InvalidInput {
            what,
            value: value.into(),
        }
    }

    /// Report something the caller named that is not there.
    pub(crate) fn not_found(what: &'static str, value: impl Into<String>) -> Self {
        Self::NotFound {
            what,
            value: value.into(),
        }
    }

    /// Refuse, with the line the operator will read.
    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self::Refused {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clock => write!(f, "system clock reports a time before the unix epoch"),
            Self::CrossDevice { from, to } => write!(
                f,
                "{} and {} are on different filesystems — sandman moves, never copies",
                from.display(),
                to.display()
            ),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidInput { what, value } => write!(f, "invalid {what}: {value:?}"),
            Self::Json { path, message } => match path {
                Some(path) => write!(f, "{}: {message}", path.display()),
                None => write!(f, "{message}"),
            },
            Self::LockHeld { path } => {
                write!(f, "commit lock held: {}", path.display())
            }
            Self::MissingEnv { name } => write!(f, "${name} is not set"),
            Self::MissingField { key, path } => {
                write!(f, "{}: missing frontmatter key `{key}`", path.display())
            }
            Self::NotFound { what, value } => write!(f, "no {what} for {value}"),
            Self::Parse { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Refused { message } => write!(f, "{message}"),
            Self::ReplacesMissing { path } => {
                write!(f, "replaces target not in bank: {}", path.display())
            }
            Self::TooManyCollisions { filename } => {
                write!(f, "too many colliding filenames for {filename}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            _ => None,
        }
    }
}
