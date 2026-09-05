//! The build stamp — what wrote a journal line.
//!
//! Every entry the journal writes carries `v=`, and `sandman version` prints
//! the same string: given a line from `~/.sandman/.trace`, the build that wrote
//! it is nameable without guessing which worktree the installed binary came
//! from. `build.rs` supplies the commit; `-dirty` means it was built over
//! uncommitted work and the commit alone does not describe it.

/// `<crate version>-<short commit>[-dirty]`, else `<crate version>-unknown`.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-", env!("SANDMAN_GIT_HASH"));

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn the_stamp_opens_with_the_crate_version_and_carries_a_build() {
        let (version, build) = VERSION.split_once('-').expect("a build suffix");
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert!(!build.is_empty(), "{VERSION}");
        // One line, one field: a stamp with a space in it would break the
        // `key=value` shape of every line that carries it.
        assert!(!VERSION.contains(' '), "{VERSION}");
    }
}
