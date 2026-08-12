//! The naming rules: `name` → slug → `<type>_<slug>.md`.
//!
//! Measured against the live banks — the rule below reproduces all 232
//! filenames with zero violations (`docs/BANK-FORMAT.md`).

/// Slugs are cut at this many characters; 39 of the 232 live files sit exactly
/// here.
pub const SLUG_MAX_CHARS: usize = 60;

/// `name` lowercased, every run of non-alphanumerics collapsed to `_`,
/// leading/trailing `_` trimmed, then cut at [`SLUG_MAX_CHARS`].
///
/// Trimming happens before the cut, so a slug may legitimately end in `_` when
/// the cut lands mid-word.
#[must_use]
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len().min(SLUG_MAX_CHARS));
    let mut separator_pending = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator_pending && !out.is_empty() {
                out.push('_');
            }
            separator_pending = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            separator_pending = true;
        }
    }
    out.truncate(truncate_index(&out, SLUG_MAX_CHARS));
    out
}

/// `<prefix>_<slug>.md` — the memory filename.
#[must_use]
pub fn filename(prefix: &str, name: &str) -> String {
    filename_nth(prefix, name, 1)
}

/// The `nth` candidate filename: `nth == 1` is the bare name, and every later
/// one carries the collision suffix `_2`, `_3`, ….
#[must_use]
pub fn filename_nth(prefix: &str, name: &str, nth: u32) -> String {
    let slug = slug(name);
    if nth <= 1 {
        format!("{prefix}_{slug}.md")
    } else {
        format!("{prefix}_{slug}_{nth}.md")
    }
}

/// Byte index that cuts `s` after `max` characters — the whole string when it
/// is shorter.
pub(crate) fn truncate_index(s: &str, max: usize) -> usize {
    s.char_indices()
        .nth(max)
        .map_or_else(|| s.len(), |(index, _)| index)
}

/// `s` cut to at most `max` characters, never splitting one.
pub(crate) fn truncate_chars(s: &str, max: usize) -> &str {
    &s[..truncate_index(s, max)]
}

#[cfg(test)]
mod tests {
    use super::{SLUG_MAX_CHARS, filename, filename_nth, slug, truncate_chars};

    #[test]
    fn collapses_runs_and_trims() {
        assert_eq!(slug("Hello, World!"), "hello_world");
        assert_eq!(slug("  --leading and trailing--  "), "leading_and_trailing");
        assert_eq!(slug("a___b"), "a_b");
        assert_eq!(
            slug("game_01: Zelda/LA-remake, ported web→Rust"),
            "game_01_zelda_la_remake_ported_web_rust"
        );
        assert_eq!(slug(""), "");
        assert_eq!(slug("---"), "");
        assert_eq!(slug("MiXeD CaSe 42"), "mixed_case_42");
    }

    #[test]
    fn truncates_at_exactly_sixty_characters() {
        let name = "a".repeat(100);
        assert_eq!(slug(&name).len(), SLUG_MAX_CHARS);
        // Sixty characters exactly: untouched.
        let exact = "b".repeat(SLUG_MAX_CHARS);
        assert_eq!(slug(&exact), exact);
        // Sixty-one: one character lost.
        let over = "c".repeat(SLUG_MAX_CHARS + 1);
        assert_eq!(slug(&over), "c".repeat(SLUG_MAX_CHARS));
    }

    #[test]
    fn keeps_a_trailing_underscore_when_the_cut_lands_on_one() {
        // Live: user_hooks_are_disabled_in_some_sessions_work_account_automation_.md
        let name = "Hooks are disabled in some sessions (work account) — automation needs non-hook fallbacks";
        let slug = slug(name);
        assert_eq!(slug.len(), SLUG_MAX_CHARS);
        assert_eq!(
            slug,
            "hooks_are_disabled_in_some_sessions_work_account_automation_"
        );
        assert_eq!(
            filename("user", name),
            "user_hooks_are_disabled_in_some_sessions_work_account_automation_.md"
        );
    }

    #[test]
    fn collision_suffixes_append_after_the_cut() {
        assert_eq!(
            filename_nth("user", "Hello World", 1),
            "user_hello_world.md"
        );
        assert_eq!(
            filename_nth("user", "Hello World", 2),
            "user_hello_world_2.md"
        );
        assert_eq!(
            filename_nth("user", "Hello World", 10),
            "user_hello_world_10.md"
        );
        let long = "z".repeat(80);
        assert_eq!(
            filename_nth("project", &long, 3),
            format!("project_{}_3.md", "z".repeat(SLUG_MAX_CHARS))
        );
    }

    #[test]
    fn truncate_chars_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("a—b—c", 3), "a—b");
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert_eq!(truncate_chars("", 3), "");
        assert_eq!(truncate_chars("🌙🌙🌙", 2), "🌙🌙");
    }
}
