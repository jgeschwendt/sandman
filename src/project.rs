//! Which directory a working directory's memories belong to.
//!
//! One repo, one bank: the checkouts of a repo are branches of the same work,
//! not separate projects, so the worktree layout collapses to the directory
//! the worktrees sit in.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// The directory a working directory's memories belong to.
///
/// A linked git worktree is a checkout, not a project: every branch of one
/// repo shares one bank, keyed by the directory the worktrees sit in — the
/// worktree's parent. Walking up from `cwd`, the first directory whose `.git`
/// is a file pointing into a `worktrees/` directory marks a linked worktree,
/// and its parent is the project; anything below the worktree root folds into
/// it. A `.git` directory (a primary checkout), a `.git` file that points
/// elsewhere (a submodule), or no `.git` at all leaves `cwd` as it is.
#[must_use]
pub fn root(cwd: &Path) -> PathBuf {
    for dir in cwd.ancestors() {
        let dot_git = dir.join(".git");
        let Ok(meta) = fs::metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            // A primary checkout owns everything below it: nothing above it
            // can reinterpret the cwd.
            break;
        }
        if !meta.is_file() {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&dot_git) else {
            continue;
        };
        if is_linked_worktree(&contents) {
            return dir
                .parent()
                .map_or_else(|| cwd.to_path_buf(), Path::to_path_buf);
        }
    }
    cwd.to_path_buf()
}

/// Whether a `.git` file's contents point into a `worktrees/` directory — the
/// shape `git worktree add` writes, and the one a submodule's `gitdir:` (into
/// `modules/`) does not have.
fn is_linked_worktree(contents: &str) -> bool {
    let Some(gitdir) = contents.trim().strip_prefix("gitdir:") else {
        return false;
    };
    Path::new(gitdir.trim())
        .components()
        .any(|component| component == Component::Normal("worktrees".as_ref()))
}

#[cfg(test)]
mod tests {
    use super::root;
    use crate::testutil::TempDir;
    use std::fs;

    #[test]
    fn a_plain_directory_is_its_own_project() {
        let temp = TempDir::new("project-plain");
        let dir = temp.path().join("plain");
        fs::create_dir_all(&dir).expect("dir");
        assert_eq!(root(&dir), dir);
    }

    #[test]
    fn a_primary_checkout_is_its_own_project_from_anywhere_inside_it() {
        let temp = TempDir::new("project-primary");
        let repo = temp.path().join("repo");
        let sub = repo.join("src").join("deep");
        fs::create_dir_all(repo.join(".git")).expect(".git dir");
        fs::create_dir_all(&sub).expect("sub");
        assert_eq!(root(&repo), repo);
        assert_eq!(root(&sub), sub);
    }

    #[test]
    fn a_linked_worktree_keys_as_its_parent() {
        let temp = TempDir::new("project-worktree");
        let worktree = temp.path().join("code").join("o").join("r").join("main");
        fs::create_dir_all(&worktree).expect("worktree");
        fs::write(
            worktree.join(".git"),
            "gitdir: /somewhere/bare/worktrees/main\n",
        )
        .expect(".git file");
        assert_eq!(root(&worktree), worktree.parent().expect("parent"));
    }

    #[test]
    fn a_directory_below_a_worktree_folds_into_the_same_project() {
        let temp = TempDir::new("project-worktree-deep");
        let worktree = temp.path().join("code").join("o").join("r").join("feat-x");
        let deep = worktree.join("src").join("verbs");
        fs::create_dir_all(&deep).expect("deep");
        fs::write(
            worktree.join(".git"),
            "gitdir: /somewhere/bare/worktrees/feat-x\n",
        )
        .expect(".git file");
        assert_eq!(root(&deep), worktree.parent().expect("parent"));
    }

    #[test]
    fn a_submodule_is_its_own_project() {
        let temp = TempDir::new("project-submodule");
        let sub = temp.path().join("repo").join("sub");
        fs::create_dir_all(&sub).expect("sub");
        fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").expect(".git file");
        assert_eq!(root(&sub), sub);
    }

    #[test]
    fn the_worktree_is_met_before_a_primary_checkout_above_it() {
        let temp = TempDir::new("project-nested");
        let parent = temp.path().join("code").join("o").join("r");
        let worktree = parent.join("main");
        let deep = worktree.join("src");
        fs::create_dir_all(&deep).expect("deep");
        fs::create_dir_all(parent.join(".git")).expect("parent .git dir");
        fs::write(
            worktree.join(".git"),
            "gitdir: /somewhere/bare/worktrees/main\n",
        )
        .expect(".git file");
        assert_eq!(root(&deep), parent);
    }
}
