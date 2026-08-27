//! sandman — memory engine for Claude sessions.
//!
//! This crate is the single format authority for the memory banks. Slugging,
//! `replaces` archiving into `_archive/`, collision suffixes and `MEMORY.md`
//! regeneration live here and nowhere else; every writer — the CLI verbs, the
//! dream pass, any future client — goes through [`commit_memory`].
//!
//! The format is documented in `docs/BANK-FORMAT.md`, measured from the live
//! banks: those files are the regression suite, and parse → render is
//! byte-identical for every one of them.

// stele:landmark format-authority
pub mod commit;

pub mod bank;
pub mod cli;
pub mod consensus;
pub mod error;
pub mod hook;
pub mod journal;
pub mod lock;
pub mod memory;
pub mod mind;
pub mod paths;
pub mod slug;
pub mod time;
pub mod transcript;
pub mod verbs;

mod atomic;
mod json;

#[cfg(test)]
mod testutil;

pub use crate::bank::Bank;
pub use crate::commit::{CommitOutcome, CommitRequest, archive_memory, commit_memory};
pub use crate::error::{Error, Result};
pub use crate::memory::{Frontmatter, MemoryFile, MemoryType};
pub use crate::time::Timestamp;
