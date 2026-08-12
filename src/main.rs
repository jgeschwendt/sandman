//! The `sandman` binary — one dispatcher, nothing else.

use std::process::ExitCode;

fn main() -> ExitCode {
    sandman::cli::main()
}
