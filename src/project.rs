//! Which directory a working directory's memories belong to.
//!
//! One repo, one bank: the checkouts of a repo are branches of the same work,
//! not separate projects, so the worktree layout collapses to the directory
//! the worktrees sit in.

use std::fs;
use std::path::{Path, PathBuf};

/// The directory a working directory's memories belong to.
///
/// A linked git worktree is a checkout, not a project: every branch of one
/// repo shares one bank, keyed by the directory the worktrees sit in — the
/// worktree's parent. Walking up from `cwd`, the first directory whose `.git`
/// is a file naming `<common>/worktrees/<name>` marks a linked worktree, and
/// its parent is the project; anything below the worktree root folds into it.
/// A worktree nested in another checkout of the same repo (Claude Code's
/// `.claude/worktrees/<name>`) folds into that checkout's project — the outer
/// worktree's parent, or a primary checkout whose `.git` is `<common>`. A
/// `cwd` that no longer exists, below a directory whose linked worktrees all
/// belong to one repo, was one of them and folds into that repo's project;
/// beside worktrees of two repos it stays as it is. That leaves a deleted
/// plain directory beside one repo's worktrees folded in, and a removed
/// `.claude/worktrees/<name>` with no sibling keyed as itself: nothing on disk
/// tells either from a vanished worktree. Otherwise a `.git` directory (a
/// primary checkout), a `.git` file naming anything else (a submodule), or no
/// `.git` at all leaves `cwd` as it is.
///
/// `cwd` is taken in its component form first — a trailing `/` or a `.`
/// names the same directory, and a bank key is the path's bytes — so the CLI,
/// the hooks and the MCP server key one directory as one bank.
#[must_use]
pub fn root(cwd: &Path) -> PathBuf {
    let cwd = &cwd.components().collect::<PathBuf>();
    if !cwd.exists()
        && let Some(live) = cwd.ancestors().find(|dir| dir.exists())
    {
        let mut worktrees = fs::read_dir(live)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| Some((common(&entry.path())?, entry.path())));
        if let Some((repo, sibling)) = worktrees.next()
            && worktrees.all(|(other, _)| other == repo)
        {
            return root(&sibling);
        }
    }
    let mut project = None;
    for dir in cwd.ancestors() {
        let dot_git = dir.join(".git");
        match (common(dir), &project) {
            (Some(common), outer) if outer.as_ref().is_none_or(|(_, outer)| *outer == common) => {
                project = Some((dir.parent().unwrap_or(dir), common));
            }
            (None, Some((_, outer))) if fs::canonicalize(&dot_git).is_ok_and(|g| g == *outer) => {
                return dir.to_path_buf();
            }
            // A primary checkout, a submodule, or another repo's worktree owns
            // everything below it: nothing above it can reinterpret the cwd.
            _ if dot_git.exists() => break,
            _ => {}
        }
    }
    project.map_or_else(|| cwd.clone(), |(dir, _)| dir.to_path_buf())
}

/// The common git dir of the linked worktree at `dir` — its `.git` a file
/// naming `<common>/worktrees/<name>`, the shape `git worktree add` writes and
/// a submodule's `gitdir:` (into `modules/`) does not have.
fn common(dir: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(dir.join(".git")).ok()?;
    let gitdir = dir.join(contents.trim().strip_prefix("gitdir:")?.trim());
    let common = gitdir.parent()?;
    (common.file_name()? == "worktrees").then(|| {
        let common = common.parent().unwrap_or(common);
        fs::canonicalize(common).unwrap_or_else(|_| common.to_path_buf())
    })
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
        // A trailing separator or a `.` names the same directory, so the same bank.
        // `PathBuf`'s `==` compares components; a bank key is the bytes.
        let spelled = root(format!("{}/./", dir.display()).as_ref());
        assert_eq!(spelled.as_os_str(), dir.as_os_str());
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

    #[test]
    fn a_vanished_worktree_folds_into_its_siblings_project() {
        let temp = TempDir::new("project-vanished");
        let parent = temp.path().join("code").join("o").join("r");
        fs::create_dir_all(parent.join("main")).expect("main");
        fs::write(
            parent.join("main").join(".git"),
            "gitdir: /somewhere/bare/worktrees/main\n",
        )
        .expect(".git file");
        assert_eq!(root(&parent.join("feat").join("src")), parent);
        // Beside worktrees of two repos, a vanished plain directory was neither.
        fs::create_dir_all(parent.join("other")).expect("other");
        fs::write(
            parent.join("other").join(".git"),
            "gitdir: /elsewhere/repo/.git/worktrees/other\n",
        )
        .expect(".git file");
        let scratch = parent.join("scratch");
        assert_eq!(root(&scratch), scratch);
    }

    #[test]
    fn a_submodule_inside_a_worktree_is_its_own_project() {
        let temp = TempDir::new("project-worktree-submodule");
        let worktree = temp.path().join("code").join("o").join("r").join("main");
        let sub = worktree.join("vendor").join("lib");
        fs::create_dir_all(&sub).expect("sub");
        fs::write(
            worktree.join(".git"),
            "gitdir: /somewhere/bare/worktrees/main\n",
        )
        .expect(".git file");
        fs::write(
            sub.join(".git"),
            "gitdir: /somewhere/bare/worktrees/main/modules/vendor/lib\n",
        )
        .expect("submodule .git file");
        assert_eq!(root(&sub), sub);
    }

    #[test]
    fn a_worktree_nested_in_a_checkout_of_its_repo_keys_as_the_repo() {
        let temp = TempDir::new("project-worktree-nested");
        let parent = temp.path().join("code").join("o").join("r");
        let fix = parent
            .join("main")
            .join(".claude")
            .join("worktrees")
            .join("fix");
        fs::create_dir_all(&fix).expect("fix");
        fs::write(
            parent.join("main").join(".git"),
            "gitdir: /somewhere/bare/worktrees/main\n",
        )
        .expect(".git file");
        fs::write(fix.join(".git"), "gitdir: /somewhere/bare/worktrees/fix\n").expect(".git file");
        assert_eq!(root(&fix), parent);
        let proj = temp.path().join("proj");
        let fix = proj.join(".claude").join("worktrees").join("fix");
        fs::create_dir_all(proj.join(".git").join("worktrees").join("fix")).expect(".git dir");
        fs::create_dir_all(&fix).expect("fix");
        let gitdir = proj.join(".git").join("worktrees").join("fix");
        fs::write(fix.join(".git"), format!("gitdir: {}\n", gitdir.display())).expect(".git file");
        assert_eq!(root(&fix), proj);
    }
}
