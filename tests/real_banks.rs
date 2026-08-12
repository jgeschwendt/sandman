//! The live banks are the regression suite: every memory file under
//! `$ORRERY_MEMORY` (default `~/.orrery/memory`) must round-trip byte-for-byte
//! through the crate's parser, and each bank's `MEMORY.md` is regenerated in
//! memory and compared against the on-disk one.
//!
//! Strictly read-only — nothing here writes inside the memory root. Ignored by
//! default because it depends on the operator's machine: run it with
//! `cargo test -- --ignored`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sandman::{Bank, MemoryFile};

/// How many differing index lines are printed per bank.
const DIFF_SAMPLE: usize = 4;

/// The live memory root: `$ORRERY_MEMORY`, else `$HOME/.orrery/memory`.
fn memory_root() -> Option<PathBuf> {
    if let Some(root) = env::var_os("ORRERY_MEMORY") {
        return Some(PathBuf::from(root));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".orrery").join("memory"))
}

/// Bank directories: every child of the root that is not `_`- or `.`-prefixed.
fn banks(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .expect("read the memory root")
        .filter_map(|entry| {
            let entry = entry.expect("read a memory-root entry");
            let name = entry.file_name();
            let name = name.to_str()?;
            if !entry.file_type().expect("file type").is_dir()
                || name.starts_with('_')
                || name.starts_with('.')
            {
                return None;
            }
            Some(entry.path())
        })
        .collect();
    dirs.sort();
    dirs
}

/// A unified-style summary of the first differing lines.
fn summarize(disk: &str, generated: &str) -> Vec<String> {
    let disk: Vec<&str> = disk.lines().collect();
    let generated: Vec<&str> = generated.lines().collect();
    let mut out = Vec::new();
    for index in 0..disk.len().max(generated.len()) {
        let left = disk.get(index).copied();
        let right = generated.get(index).copied();
        if left == right {
            continue;
        }
        if out.len() >= DIFF_SAMPLE * 2 {
            out.push("  …".to_owned());
            break;
        }
        if let Some(left) = left {
            out.push(format!("@{} -{left}", index + 1));
        }
        if let Some(right) = right {
            out.push(format!("@{} +{right}", index + 1));
        }
    }
    out
}

#[test]
#[ignore = "reads the operator's live ~/.orrery/memory banks"]
fn every_live_memory_file_round_trips() {
    let Some(root) = memory_root() else {
        panic!("neither ORRERY_MEMORY nor HOME is set");
    };
    assert!(
        root.is_dir(),
        "memory root {} is not a directory",
        root.display()
    );

    let mut files = 0_usize;
    let mut index_matches = 0_usize;
    let mut index_differs = Vec::new();

    let banks = banks(&root);
    assert!(!banks.is_empty(), "no banks under {}", root.display());

    for dir in &banks {
        let bank = Bank::at(dir.clone());
        for name in bank.memory_filenames().expect("list the bank") {
            let path = dir.join(&name);
            let text = fs::read_to_string(&path).expect("read a memory file");
            let parsed = match MemoryFile::parse(&text) {
                Ok(parsed) => parsed,
                Err(error) => panic!("{}: {error}", path.display()),
            };
            assert_eq!(
                parsed.render(),
                text,
                "{} did not round-trip byte-for-byte",
                path.display()
            );
            files += 1;
        }

        let generated = bank.render_index().expect("regenerate the index");
        let index_path = bank.index_path();
        let Ok(disk) = fs::read_to_string(&index_path) else {
            index_differs.push((index_path, vec!["  (no MEMORY.md on disk)".to_owned()]));
            continue;
        };
        if disk == generated {
            index_matches += 1;
        } else {
            index_differs.push((index_path, summarize(&disk, &generated)));
        }
    }

    // MEMORY.md differences are reported, never fatal: the on-disk indexes were
    // written by orrery and may predate the current truncation rule.
    println!(
        "round-tripped {files} memory files across {} banks",
        banks.len()
    );
    println!(
        "MEMORY.md: {index_matches} regenerated identically, {} differ",
        index_differs.len()
    );
    for (path, lines) in &index_differs {
        println!("--- {} (disk)", path.display());
        println!("+++ {} (regenerated)", path.display());
        for line in lines {
            println!("{line}");
        }
    }

    assert!(files > 0, "no memory files found under {}", root.display());
}
