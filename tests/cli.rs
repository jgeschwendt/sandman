//! The dispatcher, end to end: the built binary against a temp data root.
//!
//! Every run here points `$SANDMAN_ROOT` and `$HOME` at a temp directory, so
//! nothing touches the operator's `~/.sandman` or `~/.claude`.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Disambiguates directories created within the same process.
static COUNTER: AtomicU32 = AtomicU32::new(0);

const SID: &str = "aaaabbbb-cccc-dddd-eeee-ffff00001111";
/// The Claude Code project the fabricated sessions run under.
const PROJECT: &str = "-Users-you-code";

/// A fabricated machine: `$HOME` with `.claude` and `.sandman` inside it.
struct Machine {
    home: PathBuf,
}

impl Machine {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let home = env::temp_dir().join(format!(
            "sandman-cli-{label}-{}-{serial}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&home).expect("create the fabricated home");
        Self { home }
    }

    fn root(&self) -> PathBuf {
        self.home.join(".sandman")
    }

    fn projects(&self) -> PathBuf {
        self.home.join(".claude").join("projects")
    }

    fn project(&self) -> PathBuf {
        self.projects().join(PROJECT)
    }

    /// Seed a transcript, aged out of the live window.
    fn transcript(&self, lines: &[&str]) -> PathBuf {
        let project = self.project();
        fs::create_dir_all(&project).expect("project dir");
        let path = project.join(format!("{SID}.jsonl"));
        fs::write(&path, format!("{}\n", lines.join("\n"))).expect("transcript");
        fs::File::open(&path)
            .expect("open")
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .expect("age the transcript");
        path
    }

    /// An executable `/bin/sh` script under the fabricated home. This is how
    /// the tests stand in for `claude`: no test ever runs the real one.
    fn script(&self, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;

        let path = self.home.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_stdin(args, "")
    }

    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        self.run_with(args, stdin, &[])
    }

    fn run_with(&self, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sandman"))
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("SANDMAN_ROOT", self.root())
            .env("PATH", env::var("PATH").unwrap_or_default())
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sandman");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("wait for sandman")
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("an exit code")
}

#[test]
fn the_usage_screen_lists_the_verbs() {
    let machine = Machine::new("usage");
    let output = machine.run(&["--help"]);
    assert_eq!(code(&output), 0);
    for verb in ["remember", "take", "recall", "forget", "dream", "reflect"] {
        assert!(stdout(&output).contains(verb), "usage omits {verb}");
    }
}

#[test]
fn an_unknown_verb_is_a_usage_failure() {
    let machine = Machine::new("unknown-verb");
    let output = machine.run(&["dissolve"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("unknown verb `dissolve`"));
    assert!(stdout(&output).is_empty());

    let no_verb = machine.run(&[]);
    assert_eq!(code(&no_verb), 2);
    assert!(stderr(&no_verb).contains("no verb"));
}

#[test]
fn dream_and_reflect_run_on_an_empty_root() {
    let machine = Machine::new("empty-passes");
    // No queue, no banks: both passes are successful no-ops. Neither can
    // reach a model, because neither has anything to ask about.
    for args in [vec!["dream"], vec!["dream", "--now"]] {
        let output = machine.run_with(&args, "", &[("SANDMAN_CLAUDE_BIN", "/nonexistent/claude")]);
        assert_eq!(code(&output), 0, "{args:?}: {}", stderr(&output));
        assert!(stderr(&output).contains("dreamt 0 pointer(s)"), "{args:?}");
    }

    let output = machine.run_with(
        &["reflect"],
        "",
        &[("SANDMAN_CLAUDE_BIN", "/nonexistent/claude")],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let written: Vec<PathBuf> = stdout(&output).lines().map(PathBuf::from).collect();
    assert_eq!(written.len(), 2);
    assert!(written[0].is_file(), "the day page");
    assert_eq!(written[1], machine.root().join("log").join("INDEX.md"));
    assert!(
        fs::read_to_string(&written[0])
            .expect("day page")
            .starts_with("# 20")
    );
    assert!(stderr(&output).contains("swept 0 pointer(s)"));
}

#[test]
fn dream_runs_the_configured_minds_and_commits_on_agreement() {
    let machine = Machine::new("dream-minds");
    let archived = machine.root().join("archive").join("claude");
    fs::create_dir_all(&archived).expect("archive dir");
    // The name `take` builds: the claude project slug the minds file under.
    let transcript = archived.join(format!("2026-08-11-120000-projects-{PROJECT}-{SID}.jsonl"));
    fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            r#"{"type":"user","message":{"content":"where does recall get its short-term surface"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"from the .recent pointers"}]}}"#
        ),
    )
    .expect("archived transcript");

    let recent = machine.root().join("memories").join(".recent");
    fs::create_dir_all(&recent).expect("recent dir");
    fs::write(
        recent.join(format!("{SID}.json")),
        format!(
            "{{\"archived\":\"{}\",\"cwd\":\"{}\",\"ended\":\"2026-08-11T12:00:00Z\",\"title\":\"the recall surface\",\"highlights\":[]}}\n",
            transcript.display(),
            machine.home.join("code").display()
        ),
    )
    .expect("pointer");

    // Every mind's model id is recorded, and two of the three agree.
    let seen = machine.home.join("models.txt");
    let bank = bank_key(&machine.home.join("code"));
    let proposal = |name: &str, body: &str| {
        format!(
            "{{\\\"proposals\\\":[{{\\\"bank\\\":\\\"{bank}\\\",\\\"type\\\":\\\"project\\\",\\\"name\\\":\\\"{name}\\\",\\\"description\\\":\\\"where recall gets its short-term surface\\\",\\\"body\\\":\\\"{body}\\\"}}]}}"
        )
    };
    let stub = machine.script(
        "claude",
        &format!(
            concat!(
                "model=\"\"\nsid=\"\"\n",
                "while [ $# -gt 0 ]; do case \"$1\" in --model) model=\"$2\"; shift 2;; --session-id) sid=\"$2\"; shift 2;; *) shift;; esac; done\n",
                "echo \"$model $CLAUDE_MEMORY_PIPELINE $(pwd)\" >> \"{seen}\"\n",
                // Stand in for Claude Code's own transcript write.
                "mkdir -p \"{projects}/-a-slug\"\n",
                "printf '%s\\n' \"$model\" > \"{projects}/-a-slug/$sid.jsonl\"\n",
                "case \"$model\" in\n",
                "  mind-sonnet) result='{sonnet}' ;;\n",
                "  mind-fable) result='{fable}' ;;\n",
                "  *) result='I would rather not.' ;;\n",
                "esac\n",
                "printf '{{\"type\":\"result\",\"is_error\":false,\"result\":\"%s\"}}' \"$result\"\n",
            ),
            projects = machine.projects().display(),
            seen = seen.display(),
            sonnet = proposal("the recent pointers are the short term surface", "sonnet"),
            fable = proposal("the recent pointers are a short term surface", "fable"),
        ),
    );

    let output = machine.run_with(
        &["dream"],
        "",
        &[
            ("SANDMAN_CLAUDE_BIN", stub.to_str().expect("stub path")),
            ("SANDMAN_MIND_SONNET", "mind-sonnet"),
            ("SANDMAN_MIND_OPUS", "mind-opus"),
            ("SANDMAN_MIND_FABLE", "mind-fable"),
        ],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    // The overrides were honoured, every mind ran memory-blind, and every one
    // of them ran in `<root>/.dream` so its transcript lands somewhere known.
    let log = fs::read_to_string(&seen).expect("model log");
    let mut models: Vec<&str> = log
        .lines()
        .map(|line| line.split_whitespace().next().expect("a model"))
        .collect();
    models.sort_unstable();
    assert_eq!(models, ["mind-fable", "mind-opus", "mind-sonnet"]);
    for line in log.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields[1], "1", "{line}");
        assert!(fields[2].ends_with("/.dream"), "{line}");
    }

    // Each mind's transcript was moved out of the projects tree and filed
    // under the claude project the dreamt session ran in.
    let kept = machine.root().join(".dream").join(PROJECT);
    let names: Vec<PathBuf> = fs::read_dir(&kept)
        .expect("kept transcripts")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(
        names.iter().all(|name| name
            .extension()
            .is_some_and(|extension| extension == "jsonl")),
        "{names:?}"
    );
    assert!(
        !machine.projects().join("-a-slug").exists(),
        "a mind's transcript, or the slug directory it emptied, was left in the projects tree"
    );

    // The strongest agreeing tier's wording carried into the bank.
    let committed: Vec<PathBuf> = stdout(&output).lines().map(PathBuf::from).collect();
    assert_eq!(committed.len(), 1, "{}", stdout(&output));
    let memory = fs::read_to_string(&committed[0]).expect("committed memory");
    assert!(
        memory.contains("name: the recent pointers are a short term surface"),
        "{memory}"
    );
    assert!(memory.contains("minds=sonnet,fable"), "{memory}");
    assert_eq!(
        committed[0].parent(),
        Some(machine.root().join("memories").join(&bank).as_path())
    );

    // The pointer is stamped, so a second run has nothing to do.
    let pointer = fs::read_to_string(recent.join(format!("{SID}.json"))).expect("pointer");
    assert!(pointer.contains("\"dreamed\":\""), "{pointer}");
}

/// The bank key rule, duplicated here on purpose: an integration test drives
/// the binary and must not borrow the crate's own implementation to check it.
fn bank_key(cwd: &Path) -> String {
    cwd.to_str()
        .expect("a utf-8 cwd")
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

#[test]
fn remember_commits_into_the_bank_for_the_cwd() {
    let machine = Machine::new("remember");
    let output = machine.run(&[
        "remember",
        "the queue is the recall surface, not a side effect",
        "--cwd",
        "/Users/you/code",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let path = PathBuf::from(stdout(&output).trim());
    assert_eq!(
        path,
        machine
            .root()
            .join("memories")
            .join("-Users-you-code")
            .join("feedback_the_queue_is_the_recall_surface_not_a.md")
    );
    let text = fs::read_to_string(&path).expect("read the committed memory");
    assert!(text.contains("name: the queue is the recall surface, not a\n"));
    assert!(text.contains("type: feedback\n"));
    assert!(text.ends_with("the queue is the recall surface, not a side effect\n"));

    // The bank index was regenerated alongside it.
    let index = fs::read_to_string(path.with_file_name("MEMORY.md")).expect("read the index");
    assert!(index.contains("](feedback_the_queue_is_the_recall_surface_not_a.md) — "));
}

#[test]
fn take_archives_then_recall_surfaces_the_pointer() {
    let machine = Machine::new("take-recall");
    machine.transcript(&[
        r#"{"type":"attachment","cwd":"/Users/you/code"}"#,
        r#"{"type":"user","message":{"content":"port the session edges"}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"sandman remember \"pointers are the short-term surface\""}}]}}"#,
    ]);

    let taken = machine.run(&["take", SID]);
    assert_eq!(code(&taken), 0, "{}", stderr(&taken));
    let archived = PathBuf::from(stdout(&taken).trim());
    assert!(archived.is_file());
    assert!(archived.starts_with(machine.root().join("archive").join("claude")));
    assert!(!machine.project().join(format!("{SID}.jsonl")).exists());
    assert!(stderr(&taken).is_empty(), "one pointer is not a due queue");

    let pointer = machine
        .root()
        .join("memories")
        .join(".recent")
        .join(format!("{SID}.json"));
    let text = fs::read_to_string(&pointer).expect("read the pointer");
    assert!(text.contains(r#""title":"port the session edges""#));
    assert!(text.contains(r#""highlights":["pointers are the short-term surface"]"#));

    // Recall folds the pointer in, after the banks.
    let recalled = machine.run(&["recall", "--cwd", "/Users/you/code"]);
    assert_eq!(code(&recalled), 0, "{}", stderr(&recalled));
    assert!(stdout(&recalled).contains("## Recent sessions (3 days)"));
    assert!(stdout(&recalled).contains("- port the session edges · ended "));

    // …and forget destroys every copy of it.
    let forgotten = machine.run(&["forget", SID]);
    assert_eq!(code(&forgotten), 0, "{}", stderr(&forgotten));
    let destroyed: Vec<PathBuf> = stdout(&forgotten)
        .lines()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    assert_eq!(destroyed, [archived.clone(), pointer.clone()]);
    assert!(!archived.exists());
    assert!(!pointer.exists());

    let again = machine.run(&["forget", SID]);
    assert_eq!(code(&again), 1);
    assert!(stderr(&again).starts_with("sandman: no trace for "));
}

#[test]
fn the_hooks_read_their_payloads_from_stdin() {
    let machine = Machine::new("hooks");
    machine.transcript(&[
        r#"{"type":"attachment","cwd":"/Users/you/code"}"#,
        r#"{"type":"user","message":{"content":"the hook path"}}"#,
    ]);

    // SessionEnd → take.
    let payload = format!(
        r#"{{"hook_event_name":"SessionEnd","session_id":"{SID}","cwd":"/Users/you/code"}}"#
    );
    let taken = machine.run_with_stdin(&["take", "--hook"], &payload);
    assert_eq!(code(&taken), 0, "{}", stderr(&taken));
    assert!(PathBuf::from(stdout(&taken).trim()).is_file());

    // A payload with no session is a quiet no-op.
    let empty = machine.run_with_stdin(&["take", "--hook"], r#"{"cwd":"/Users/you/code"}"#);
    assert_eq!(code(&empty), 0);
    assert!(stdout(&empty).is_empty());
    assert!(stderr(&empty).is_empty());

    // A session forget already destroyed is a quiet no-op for the hook…
    let payload = format!(r#"{{"session_id":"{SID}","cwd":"/Users/you/code"}}"#);
    let gone = machine.run_with_stdin(&["take", "--hook"], &payload);
    assert_eq!(code(&gone), 0);
    assert!(stdout(&gone).is_empty());
    assert!(stderr(&gone).is_empty());
    // …and still an error when it was asked for by hand.
    let by_hand = machine.run(&["take", SID]);
    assert_eq!(code(&by_hand), 1);
    assert!(stderr(&by_hand).contains("no transcript for"));

    // A malformed payload is an error, not a panic.
    let broken = machine.run_with_stdin(&["take", "--hook"], "not json");
    assert_eq!(code(&broken), 1);
    assert!(stderr(&broken).contains("hook payload"));

    // SessionStart → recall, answered in the hook envelope.
    let recalled = machine.run_with_stdin(
        &["recall", "--hook"],
        r#"{"hook_event_name":"SessionStart","cwd":"/Users/you/code","source":"startup"}"#,
    );
    assert_eq!(code(&recalled), 0, "{}", stderr(&recalled));
    let reply = stdout(&recalled);
    assert!(reply.starts_with(r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","#));
    assert!(reply.contains(r"## Recent sessions (3 days)\n- the hook path"));
}

#[test]
fn recall_is_silent_with_nothing_to_say_and_for_the_pipeline() {
    let machine = Machine::new("recall-silent");
    let output = machine.run(&["recall", "--cwd", "/Users/you/code"]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).is_empty());

    // A bank to recall — and a memory-blind pipeline run that must not.
    let remembered = machine.run(&[
        "remember",
        "a rule worth recalling",
        "--cwd",
        "/Users/you/code",
    ]);
    assert_eq!(code(&remembered), 0, "{}", stderr(&remembered));
    assert!(
        !machine
            .run(&["recall", "--cwd", "/Users/you/code"])
            .stdout
            .is_empty()
    );
    let blind = machine.run_with(
        &["recall", "--cwd", "/Users/you/code"],
        "",
        &[("CLAUDE_MEMORY_PIPELINE", "1")],
    );
    assert_eq!(code(&blind), 0);
    assert!(stdout(&blind).is_empty());
}

#[test]
fn take_hook_stays_out_of_the_memory_pipeline() {
    let machine = Machine::new("pipeline-guard");
    machine.transcript(&[
        r#"{"type":"attachment","cwd":"/Users/you/code"}"#,
        r#"{"type":"user","message":{"content":"a dream mind's own session"}}"#,
    ]);
    let payload = format!(
        r#"{{"hook_event_name":"SessionEnd","session_id":"{SID}","cwd":"/Users/you/code"}}"#
    );

    // A dream mind's ending is not a session to keep — taking it would feed
    // the queue that spawned the mind, recursively.
    let guarded = machine.run_with(
        &["take", "--hook"],
        &payload,
        &[("CLAUDE_MEMORY_PIPELINE", "1")],
    );
    assert_eq!(code(&guarded), 0, "{}", stderr(&guarded));
    assert!(stdout(&guarded).is_empty());
    assert!(stderr(&guarded).is_empty());

    // The same ending outside the pipeline is taken as ever.
    let taken = machine.run_with_stdin(&["take", "--hook"], &payload);
    assert_eq!(code(&taken), 0, "{}", stderr(&taken));
    assert!(PathBuf::from(stdout(&taken).trim()).is_file());
}

#[test]
fn take_hook_declines_a_resume_because_a_resume_is_a_beginning() {
    let machine = Machine::new("resume-guard");
    let source = machine.transcript(&[
        r#"{"type":"attachment","cwd":"/Users/you/code"}"#,
        r#"{"type":"user","message":{"content":"the session being adopted"}}"#,
    ]);
    let payload = |reason: &str| {
        format!(
            r#"{{"hook_event_name":"SessionEnd","session_id":"{SID}","cwd":"/Users/you/code","reason":"{reason}"}}"#
        )
    };

    // Claude Code fires this on the session it is adopting. Taking it would
    // move the transcript out from under the turn about to append to it.
    let declined = machine.run_with_stdin(&["take", "--hook"], &payload("resume"));
    assert_eq!(code(&declined), 0, "{}", stderr(&declined));
    assert!(stdout(&declined).is_empty());
    assert!(stderr(&declined).is_empty());
    assert!(source.is_file(), "the transcript stays in the live set");
    assert!(!machine.root().exists(), "and nothing is written");

    // Every other reason is an ending, taken as ever.
    for reason in ["clear", "logout", "other", "prompt_input_exit"] {
        let source = machine.transcript(&[
            r#"{"type":"attachment","cwd":"/Users/you/code"}"#,
            r#"{"type":"user","message":{"content":"an ending"}}"#,
        ]);
        let taken = machine.run_with_stdin(&["take", "--hook"], &payload(reason));
        assert_eq!(code(&taken), 0, "{reason}: {}", stderr(&taken));
        assert!(PathBuf::from(stdout(&taken).trim()).is_file(), "{reason}");
        assert!(!source.exists(), "{reason}");
    }

    // Named by hand, a resume is still a take — the guard is the hook's.
    let source = machine.transcript(&[r#"{"type":"user","message":{"content":"by hand"}}"#]);
    let by_hand = machine.run(&["take", SID]);
    assert_eq!(code(&by_hand), 0, "{}", stderr(&by_hand));
    assert!(!source.exists());
}

#[test]
fn take_hook_declines_when_the_caller_says_this_is_not_an_ending() {
    let machine = Machine::new("no-take-guard");
    let source = machine.transcript(&[
        r#"{"type":"attachment","cwd":"/Users/you/code"}"#,
        r#"{"type":"user","message":{"content":"a resume turn's ending"}}"#,
    ]);
    let payload = format!(
        r#"{{"hook_event_name":"SessionEnd","session_id":"{SID}","cwd":"/Users/you/code"}}"#
    );

    // A driven resume turn ends the session it borrowed; the transcript has to
    // stay live for the next one.
    let declined = machine.run_with(&["take", "--hook"], &payload, &[("SANDMAN_NO_TAKE", "1")]);
    assert_eq!(code(&declined), 0, "{}", stderr(&declined));
    assert!(stdout(&declined).is_empty());
    assert!(stderr(&declined).is_empty());
    assert!(source.is_file());
    assert!(!machine.root().exists());

    // Any value declines.
    let declined = machine.run_with(&["take", "--hook"], &payload, &[("SANDMAN_NO_TAKE", "yes")]);
    assert_eq!(code(&declined), 0, "{}", stderr(&declined));
    assert!(source.is_file());

    // A session named by hand means it, guard or not.
    let by_hand = machine.run_with(&["take", SID], "", &[("SANDMAN_NO_TAKE", "1")]);
    assert_eq!(code(&by_hand), 0, "{}", stderr(&by_hand));
    assert!(!source.exists());
    assert!(PathBuf::from(stdout(&by_hand).trim()).is_file());

    // Set-but-empty is not set: the ending is an ending again.
    let source = machine.transcript(&[r#"{"type":"user","message":{"content":"a real ending"}}"#]);
    let taken = machine.run_with(&["take", "--hook"], &payload, &[("SANDMAN_NO_TAKE", "")]);
    assert_eq!(code(&taken), 0, "{}", stderr(&taken));
    assert!(!source.exists());
    assert!(PathBuf::from(stdout(&taken).trim()).is_file());
}

#[test]
fn remember_stamps_the_session_from_the_environment() {
    let machine = Machine::new("remember-session");
    let output = machine.run_with(
        &["remember", "a body", "--cwd", "/Users/you/code"],
        "",
        &[("CLAUDE_SESSION_ID", SID)],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = fs::read_to_string(stdout(&output).trim()).expect("read the memory");
    assert!(text.contains(&format!(" · {SID}\n")), "{text}");
}

#[test]
fn a_live_transcript_is_refused_with_one_line() {
    let machine = Machine::new("live");
    let path = machine.transcript(&[r#"{"type":"user","message":{"content":"still going"}}"#]);
    fs::File::open(&path)
        .expect("open")
        .set_modified(SystemTime::now())
        .expect("touch");

    let refused = machine.run(&["take", SID]);
    assert_eq!(code(&refused), 1);
    assert_eq!(stderr(&refused).lines().count(), 1);
    assert!(stderr(&refused).contains("looks live"));
    assert!(path.is_file());

    let forced = machine.run(&["take", SID, "--force"]);
    assert_eq!(code(&forced), 0, "{}", stderr(&forced));
    assert!(!path.exists());
}

#[test]
fn a_full_queue_spawns_a_detached_dream() {
    let machine = Machine::new("queue");
    let recent = machine.root().join("memories").join(".recent");
    fs::create_dir_all(&recent).expect("recent dir");
    for index in 0..9 {
        fs::write(
            recent.join(format!("older-{index}.json")),
            format!("{{\"ended\":\"2026-08-0{}T12:00:00Z\"}}\n", index + 1),
        )
        .expect("pointer");
    }
    machine.transcript(&[r#"{"type":"user","message":{"content":"the tenth"}}"#]);

    // The spawned dream reaches a stub, never the real binary.
    let stub = machine.script(
        "claude",
        "printf '%s' '{\"type\":\"result\",\"is_error\":false,\"result\":\"{\\\"proposals\\\":[]}\"}'\n",
    );
    let taken = machine.run_with(
        &["take", SID],
        "",
        &[("SANDMAN_CLAUDE_BIN", stub.to_str().expect("stub path"))],
    );
    assert_eq!(code(&taken), 0, "{}", stderr(&taken));
    assert!(
        stderr(&taken).starts_with("queue at 10 — dreaming in the background"),
        "{}",
        stderr(&taken)
    );

    // take returned without waiting; the dream lands in its own log.
    let log_dir = machine.root().join("log");
    let mut lines = String::new();
    for _ in 0..200 {
        lines = fs::read_dir(&log_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| name.starts_with("dream-"))
                    })
                    .map(|entry| fs::read_to_string(entry.path()).unwrap_or_default())
                    .collect::<String>()
            })
            .unwrap_or_default();
        if lines.matches(" dream sid=").count() >= 10 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(lines.matches(" dream sid=").count(), 10, "{lines}");
    assert!(lines.contains("groups=0"), "{lines}");
    assert!(
        lines.contains("dreamt 10 pointer(s)"),
        "the child's own stderr is in the log: {lines}"
    );
}

#[test]
fn bad_flags_are_usage_failures_and_write_nothing() {
    let machine = Machine::new("bad-flags");
    for args in [
        vec!["remember"],
        vec!["remember", "body", "--type", "nonsense"],
        vec!["remember", "body", "--name"],
        vec!["remember", "body", "--nope"],
        vec!["take"],
        vec!["take", SID, "--hook"],
        vec!["forget"],
        vec!["recall", "positional"],
    ] {
        let output = machine.run(&args);
        assert_eq!(code(&output), 2, "{args:?}");
        assert!(stderr(&output).starts_with("sandman: "), "{args:?}");
    }
    assert!(!Path::new(&machine.root()).exists());
}
