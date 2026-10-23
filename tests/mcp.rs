//! `sandman mcp`, end to end: the built binary with a scripted stdin.
//!
//! The transport is the thing under test here — the handlers have their own
//! unit tests inside the crate. So every run drives the real process the way a
//! client does: a conversation on stdin, one JSON object per line back on
//! stdout, and nothing else on stdout ever.
//!
//! `$SANDMAN_ROOT` and `$HOME` point at a temp directory in every run, so
//! nothing here touches the operator's `~/.sandman` or `~/.claude`. The one
//! bank a test needs is seeded by running the binary's own `remember` first:
//! the commit path is the format authority, and a test that hand-rolled a bank
//! would be checking its own fabrication.

mod common;

use std::fs;
use std::path::Path;
use std::process::Output;

use common::{Machine, bank_key, code, feed, stderr, stdout};

/// The bank the scripted `remember` call writes into — a directory that does
/// not exist on this machine, so the run cannot depend on one that does.
const WRITTEN_BANK: &str = "-Users-you-code";

/// The body that scripted call commits. It is also the string the journal must
/// never carry: a memory belongs in its bank, not doubled into the log.
const WRITTEN_BODY: &str = "the queue is the recall surface, not a side effect";

/// The filename the commit path gives [`WRITTEN_BODY`] — the body's first eight
/// words, slugged.
const WRITTEN_FILE: &str = "feedback_the_queue_is_the_recall_surface_not_a.md";

/// The body of the memory seeded into the home bank before the conversation
/// starts, so `recall` has something to compose.
const SEED_BODY: &str = "the archive is append-only and the pointer is the queue";

/// A fragment of [`SEED_BODY`] that no journal line may contain: the recall
/// payload is what the log is forbidden to quote.
const SEED_FRAGMENT: &str = "append-only";

impl Machine {
    /// Seed the home bank through the binary's own `remember`.
    fn seed(&self) {
        let home = self.home.display().to_string();
        let output = self.run_with_stdin(&["remember", SEED_BODY, "--cwd", &home], "");
        assert_eq!(code(&output), 0, "seed: {}", stderr(&output));
    }

    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        feed(self.sandman(args).env("SANDMAN_ROOT", self.root()), stdin)
    }
}

/// The whole conversation, in the order a client holds it: the handshake, the
/// three tools, then the four ways a caller can be wrong, then a ping.
///
/// The `initialize` line is the legacy handshake Claude Code 2.1.268 actually
/// sends, key order and all — `method` first and `jsonrpc` last — because a
/// server that only answered a tidier one would fail against the real client.
fn script(home: &Path) -> String {
    let lines = [
        format!(
            "{}{}{}",
            r#"{"method":"initialize","params":{"protocolVersion":"2025-11-25","#,
            r#""capabilities":{"roots":{"listChanged":true},"elicitation":{}},"#,
            r#""clientInfo":{"name":"claude-code","title":"Claude Code","version":"2.1.268"}},"jsonrpc":"2.0","id":0}"#,
        ),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_owned(),
        format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"recall","arguments":{{"cwd":"{}"}}}}}}"#,
            home.display()
        ),
        format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"remember","arguments":{{"body":"{WRITTEN_BODY}","cwd":"/Users/you/code"}}}}}}"#
        ),
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"banks"}}"#.to_owned(),
        "{not json".to_owned(),
        r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"forget","arguments":{}}}"#
            .to_owned(),
        format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"remember","arguments":{{"body":"b","bank":"{WRITTEN_BANK}","cwd":"/Users/you/code"}}}}}}"#
        ),
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"remember","arguments":{"body":"   "}}}"#
            .to_owned(),
        r#"{"jsonrpc":"2.0","id":"p","method":"ping"}"#.to_owned(),
    ];
    format!("{}\n", lines.join("\n"))
}

#[test]
fn the_server_holds_a_whole_conversation_and_exits_0_at_eof() {
    let machine = Machine::new("conversation");
    machine.seed();
    let stamp = stdout(&machine.run_with_stdin(&["version"], ""))
        .trim()
        .to_owned();

    let output = machine.run_with_stdin(&["mcp"], &script(&machine.home));
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    // Nothing but JSON-RPC on stdout, and nothing at all on stderr: one stray
    // line on either stream is a client that cannot parse the stream again.
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
    let answered = stdout(&output);
    let lines: Vec<&str> = answered.lines().collect();
    assert_eq!(lines.len(), 11, "{answered}");
    for line in &lines {
        assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
        assert!(line.contains(r#""jsonrpc":"2.0""#), "{line}");
    }

    // 1 · the handshake, answered with the version the client asked for.
    assert!(lines[0].contains(r#""id":0"#), "{}", lines[0]);
    assert!(
        lines[0].contains(r#""protocolVersion":"2025-11-25""#),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains(r#""name":"sandman""#), "{}", lines[0]);
    assert!(
        lines[0].contains(r#""capabilities":{"tools":{}}"#),
        "{}",
        lines[0]
    );
    // The `serverInfo` version is the stamp `sandman version` prints, so a
    // journal line can be read against the build that answered.
    assert!(!stamp.is_empty());
    assert!(
        lines[0].contains(&format!(r#""version":"{stamp}""#)),
        "{}",
        lines[0]
    );

    // 2 · the tools, in the order of use: read, write, look up a key.
    let listed = lines[1];
    let mut at = 0;
    for tool in ["recall", "remember", "banks"] {
        let found = listed[at..]
            .find(&format!(r#""name":"{tool}""#))
            .unwrap_or_else(|| panic!("{tool} is not listed after {at}: {listed}"));
        at += found + 1;
    }

    // 3 · recall: a tool result, and the seeded bank inside its text.
    assert!(lines[2].contains(r#""isError":false"#), "{}", lines[2]);
    let seeded = bank_key(&machine.home);
    assert!(
        lines[2].contains(&seeded),
        "{seeded} missing from {}",
        lines[2]
    );

    // 4 · remember: the file and the index the CLI would have written, and the
    // outcome reported back through the escaped result text.
    assert!(
        lines[3].contains(r#"\"outcome\":\"created\""#),
        "{}",
        lines[3]
    );
    let bank = machine.root().join("memories").join(WRITTEN_BANK);
    let committed = fs::read_to_string(bank.join(WRITTEN_FILE)).expect("the committed memory");
    assert!(
        committed.ends_with(&format!("{WRITTEN_BODY}\n")),
        "{committed}"
    );
    let index = fs::read_to_string(bank.join("MEMORY.md")).expect("the index");
    assert!(index.contains(&format!("]({WRITTEN_FILE}) — ")), "{index}");

    // 5 · banks: both banks, the seeded one and the one just written into.
    assert!(lines[4].contains(&seeded), "{}", lines[4]);
    assert!(lines[4].contains(WRITTEN_BANK), "{}", lines[4]);

    // 6 · a line that is not JSON: one parse error with a null id, and the
    // conversation carries on.
    assert_eq!(
        lines[5],
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#
    );

    // 7 · a method this server does not have.
    assert!(lines[6].contains(r#""id":5"#), "{}", lines[6]);
    assert!(lines[6].contains(r#""code":-32601"#), "{}", lines[6]);

    // 8 · an unknown tool, and 9 · bank and cwd together: the caller's error,
    // so a protocol error rather than a tool result.
    assert!(lines[7].contains(r#""id":6"#), "{}", lines[7]);
    assert!(lines[7].contains(r#""code":-32602"#), "{}", lines[7]);
    assert!(lines[8].contains(r#""id":7"#), "{}", lines[8]);
    assert!(lines[8].contains(r#""code":-32602"#), "{}", lines[8]);

    // 10 · a body the commit path refuses: a result the model reads and
    // recovers from, never a protocol error.
    assert!(lines[9].contains(r#""id":8"#), "{}", lines[9]);
    assert!(lines[9].contains(r#""isError":true"#), "{}", lines[9]);
    assert!(!lines[9].contains(r#""error""#), "{}", lines[9]);

    // 11 · ping, whose whole answer is that the server is there.
    assert_eq!(lines[10], r#"{"jsonrpc":"2.0","id":"p","result":{}}"#);
}

#[test]
fn work_before_the_handshake_is_refused_and_the_server_still_exits_0() {
    let machine = Machine::new("before-initialize");
    let output = machine.run_with_stdin(
        &["mcp"],
        "\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let answered = stdout(&output);
    let lines: Vec<&str> = answered.lines().collect();
    // The blank line said nothing; the request said something too early.
    assert_eq!(lines.len(), 1, "{answered}");
    assert!(lines[0].contains(r#""id":1"#), "{}", lines[0]);
    assert!(lines[0].contains(r#""code":-32600"#), "{}", lines[0]);
    assert!(lines[0].contains("not initialized"), "{}", lines[0]);
}

#[test]
fn the_journal_carries_one_line_per_message_and_never_a_body() {
    let machine = Machine::new("journal");
    machine.seed();
    assert_eq!(
        code(&machine.run_with_stdin(&["mcp"], &script(&machine.home))),
        0
    );

    let journal = machine.journal("mcp");
    let lines: Vec<&str> = journal.lines().collect();
    // The start, the eleven messages, the notification, and the ending.
    assert_eq!(lines.len(), 14, "{journal}");
    for expected in [
        "served start",
        "served method=initialize client=claude-code/2.1.268 requested=2025-11-25 \
protocol=2025-11-25",
        "served method=notifications/initialized",
        "served method=tools/list",
        "served method=tools/call tool=recall ",
        "served method=tools/call tool=remember ",
        "served method=tools/call tool=banks ",
        "error method=none code=-32700",
        "error method=resources/list code=-32601",
        "error method=tools/call code=-32602",
        "failed method=tools/call tool=remember kind=InvalidInput",
        "served method=ping",
        "served eof",
    ] {
        assert!(
            journal.contains(expected),
            "{expected} missing from {journal}"
        );
    }
    // Every line is stamped and carries the build that wrote it.
    for line in &lines {
        assert!(line.contains(" pid=") && line.contains(" v="), "{line}");
    }

    // The shape, and never the content: not the body that was committed, not a
    // character of what recall composed, and not the description of either.
    assert!(!journal.contains("recall surface"), "{journal}");
    assert!(!journal.contains(SEED_FRAGMENT), "{journal}");
    assert!(!journal.contains(SEED_BODY), "{journal}");
}

#[test]
fn the_verb_takes_no_arguments_and_the_usage_screen_names_it() {
    let machine = Machine::new("usage");
    let extra = machine.run_with_stdin(&["mcp", "extra"], "");
    assert_eq!(code(&extra), 2);
    assert!(
        stderr(&extra).contains("mcp takes no arguments"),
        "{}",
        stderr(&extra)
    );
    assert!(stdout(&extra).is_empty());

    let flag = machine.run_with_stdin(&["mcp", "--nonsense"], "");
    assert_eq!(code(&flag), 2);

    // A stdin that cannot be read (a directory) is an error, not an ending.
    let unread = machine
        .sandman(&["mcp"])
        .stdin(fs::File::open(&machine.home).expect("open home"))
        .output()
        .expect("run");
    assert_eq!(code(&unread), 1, "{}", stderr(&unread));

    let usage = machine.run_with_stdin(&["--help"], "");
    assert_eq!(code(&usage), 0, "{}", stderr(&usage));
    assert!(stdout(&usage).contains("  mcp\n"), "{}", stdout(&usage));

    // `mcp --help` is the same screen, and a success.
    let helped = machine.run_with_stdin(&["mcp", "--help"], "");
    assert_eq!(code(&helped), 0, "{}", stderr(&helped));
    assert_eq!(stdout(&helped), stdout(&usage));
}
