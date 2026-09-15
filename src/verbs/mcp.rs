//! `mcp` — the banks as a tool server.
//!
//! Three tools, the smallest surface a caller that is not this Mac can use:
//! `recall` composes what a session would have been primed with, `remember`
//! commits one memory through the commit path, `banks` says which banks exist
//! and which of them still name a directory. `take`, `dream`, `reflect` and
//! `forget` stay off the wire — they are lifecycle verbs with no remote caller,
//! and `forget` is the privacy ending (`docs/MCP.md`).
//!
//! The handlers here are plain functions over a [`Context`]: roots in, a
//! `Value` and a journal fragment out, nothing read from the environment and
//! nothing written to stdout. The stdio transport is a thin loop around them,
//! which is the whole point — an HTTP transport later is a second loop, not a
//! second server, and neither one can drift from the other's semantics because
//! there is only one set of semantics to drift from.
//!
//! Every write goes through `verbs::remember::remember` and therefore through
//! `commit.rs`: this module holds no format knowledge of its own, not the
//! slugging, not the index, not the collision suffix it merely reports.
//!
//! [`serve`] is the loop: one JSON-RPC message per line in, one response line
//! out, sequentially, until the input ends. It journals a line per message to
//! `<root>/.trace/mcp-<date>.log` — the method, the tool, the shape of the
//! answer and how long it took, and never a body, a name, a description or a
//! recall payload. Nothing but JSON-RPC reaches the output stream: one stray
//! `println!` anywhere under this verb breaks the client.

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::time::Instant;

use crate::bank::{Bank, MEMORIES_DIR_NAME};
use crate::error::Error;
use crate::json::Value;
use crate::memory::MemoryType;
use crate::time::Timestamp;
use crate::verbs::recall::BUDGET_CHARS;
use crate::verbs::remember::Remember;
use crate::version::VERSION;

/// The protocol versions this server speaks, newest first.
///
/// Negotiation is the 2025-11-25 lifecycle rule: a client asking for one of
/// these is answered with the same string, and anything else — an unknown
/// version, a missing field, a non-string — is answered with the newest.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The name the server reports in `serverInfo`, and the prefix a client puts
/// in front of every tool name.
pub const SERVER_NAME: &str = "sandman";

/// How many path components a bank key may decode to before [`is_live`] gives
/// up. A key is a directory path; nothing real is sixty-four deep, and the
/// bound is what turns a symlink cycle into a `false`.
const MAX_KEY_COMPONENTS: usize = 64;

/// The journal's stand-in for a field that could not be read. Same word the
/// CLI's own lines use, so the two logs read alike.
const NONE: &str = "none";

/// The journal's stand-in for an empty list, as in `trimmed=-`.
const EMPTY: &str = "-";

/// The four `type` values, named back at a caller that sent a fifth.
const TYPES: &str = "user, feedback, project, reference";

/// JSON-RPC `Invalid params` — a tool call whose arguments the handlers
/// refused before the verb ever ran.
const INVALID_PARAMS: i32 = -32602;

/// JSON-RPC `Invalid Request` — not an object, not `"jsonrpc":"2.0"`, no
/// string method, or a request that arrived before `initialize` was answered.
const INVALID_REQUEST: i32 = -32600;

/// The verb the loop journals under: `<root>/.trace/mcp-<date>.log`.
const JOURNAL_VERB: &str = "mcp";

/// The JSON-RPC version every message carries, in both directions.
const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC `Method not found`.
const METHOD_NOT_FOUND: i32 = -32601;

/// JSON-RPC `Parse error` — a line that is not JSON, or not UTF-8.
const PARSE_ERROR: i32 = -32700;

/// What the model reads before calling `banks`.
const BANKS_DESCRIPTION: &str = concat!(
    "Every memory bank on this machine: its key (the working directory with ",
    "every non-alphanumeric character replaced by -), whether that directory ",
    "still exists (live: false means the bank should be retired, not written ",
    "to), and how many memories it holds. Use a key from here as bank for ",
    "remember, or the directory it names as cwd for recall.",
);

/// What the model reads before calling `recall`.
const RECALL_DESCRIPTION: &str = concat!(
    "What past sessions know for a working directory, composed exactly as a ",
    "Claude Code session is primed at start: the directory's memory bank, its ",
    "ancestors' banks, the last three days of session pointers, the voyage log ",
    "tail and the tool index, inside one 9,000-character budget. Not every ",
    "surface arrives — the budget drops the cheap ones and cuts banks back to ",
    "one line per memory, and trimmed.sections names the surfaces it dropped ",
    "while trimmed.banks_degraded counts the banks it cut back. The voyage ",
    "log's latest entry is carried whenever the log has one: its cost is ",
    "reserved before anything else is fitted. text is the payload a session ",
    "would have read; banks, memories, pointers and trimmed say what the ",
    "budget cut. cwd defaults to the home directory.",
);

/// What the model reads before calling `remember` — the budget rule included,
/// because the nudge is the only length enforcement there is.
const REMEMBER_DESCRIPTION: &str = concat!(
    "Commit one memory into a bank now, through sandman's commit path — the ",
    "only writer of the banks. Memory is a budget: a body is a few short ",
    "sentences stating a rule, a preference, a decision and its reason, or how ",
    "a tool behaves; never counts, versions, prices, directory listings or ",
    "anything re-derivable from the tree, and never a secret. type: user (who ",
    "the operator is and how they work), feedback (a correction or preference ",
    "to honor), project (the state of a codebase), reference (how a tool or ",
    "system behaves). Pass bank (a key from the banks tool) or cwd (its ",
    "working directory), never both; the default is the home bank.",
);

/// What every handler runs over — resolved once by the caller, never the
/// environment.
pub struct Context {
    /// The data root: `$SANDMAN_ROOT` else `~/.sandman`, resolved in
    /// `paths.rs` and nowhere else.
    pub data_root: PathBuf,
    /// The home directory — the default `cwd` for both `recall` and
    /// `remember`.
    pub home: PathBuf,
    /// `$CLAUDE_SESSION_ID` when the launcher had one; the transport reads the
    /// environment, handlers never do.
    pub session_id: Option<String>,
}

/// A handler's failure.
///
/// The split is the one `docs/MCP.md` draws: the caller got the call wrong, or
/// the call was fine and the verb refused. Only the first is a protocol error;
/// the second is a result the model is meant to read and recover from.
#[derive(Debug)]
pub enum McpError {
    /// The caller's parameters were wrong — a JSON-RPC `-32602` upstream.
    InvalidParams(String),
    /// The verb ran and failed — a tool result with `isError: true` upstream.
    Verb(Error),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParams(message) => write!(f, "{message}"),
            Self::Verb(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidParams(_) => None,
            Self::Verb(source) => Some(source),
        }
    }
}

impl From<Error> for McpError {
    fn from(source: Error) -> Self {
        Self::Verb(source)
    }
}

/// What a handler answers: the tool's result object, and the journal fragment
/// describing its shape — never its content.
pub(crate) struct Answer {
    /// The journal fragment, in the `key=value` shape every verb's lines take.
    pub note: String,
    /// The tool result, rendered into the `content` text upstream.
    pub result: Value,
}

/// Serve MCP over one pair of streams until the input ends.
///
/// One JSON-RPC message per line in, one response line out, flushed as it goes:
/// a client blocks on its answer, so a buffered response is a hung session.
/// Requests are handled strictly in order and nothing is spawned — stdio is one
/// client, and the commit lock is what serializes this server against the
/// passes.
///
/// Generic over the streams so a test drives the whole protocol with a
/// `Cursor`; `cli.rs` hands it the locked stdin and stdout. EOF is a normal
/// ending, and so is a `BrokenPipe`: the client hung up, and there is nobody
/// left to read a failure about it.
pub fn serve(ctx: &Context, input: impl BufRead, output: impl Write) -> io::Result<()> {
    match converse(ctx, input, output) {
        Err(source) if source.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        outcome => outcome,
    }
}

/// The loop itself, with every write still able to fail.
fn converse(ctx: &Context, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    note(ctx, "served start");
    let mut initialized = false;
    for line in input.lines() {
        let line = match line {
            Ok(line) => line,
            // A line that is not UTF-8 cannot be JSON either, and `read_line`
            // has already stepped past it — so it is one parse error, not the
            // end of the conversation.
            Err(source) if source.kind() == io::ErrorKind::InvalidData => {
                note(ctx, &format!("error method={NONE} code={PARSE_ERROR}"));
                write_line(
                    &mut output,
                    &rpc_error(Value::Null, PARSE_ERROR, "parse error"),
                )?;
                continue;
            }
            Err(source) => return Err(source),
        };
        // Blank lines are framing, not messages: a client that pads its stream
        // has said nothing to answer.
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = respond(ctx, &mut initialized, &line) {
            write_line(&mut output, &response)?;
        }
    }
    note(ctx, "served eof");
    Ok(())
}

/// Answer one line.
///
/// `None` is a notification: receiving one is correct, and answering it would
/// be a protocol error of our own.
fn respond(ctx: &Context, initialized: &mut bool, line: &str) -> Option<Value> {
    let Ok(message) = crate::json::parse(line) else {
        note(ctx, &format!("error method={NONE} code={PARSE_ERROR}"));
        return Some(rpc_error(Value::Null, PARSE_ERROR, "parse error"));
    };
    // Anything that is not an object — a batch array included — answers `None`
    // to every lookup, so it carries no id to echo and no method to run.
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let notification = message.get("id").is_none();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        note(ctx, &format!("error method={NONE} code={INVALID_REQUEST}"));
        return Some(rpc_error(id, INVALID_REQUEST, "invalid request"));
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        note(
            ctx,
            &format!("error method={method} code={INVALID_REQUEST}"),
        );
        return Some(rpc_error(id, INVALID_REQUEST, "invalid request"));
    }

    // Ordered, not sorted: the guards are the dispatch. `initialize` and `ping`
    // stand in front of the handshake check, notifications in front of every
    // method that would otherwise answer one.
    match method {
        "initialize" if !notification => {
            // A second `initialize` is answered the same way. The handshake is
            // the client's to repeat — a reconnecting one that had to ask twice
            // should not find a server that has decided it already knows.
            *initialized = true;
            Some(initialize(ctx, id, message.get("params")))
        }
        // The client's half of the handshake: the one notification that is part
        // of the protocol rather than beside it.
        "notifications/initialized" => {
            note(ctx, "served method=notifications/initialized");
            None
        }
        _ if notification => {
            note(ctx, &format!("ignored method={method}"));
            None
        }
        // Allowed before the handshake: a client that pings a server it has not
        // initialized is asking whether it is there at all.
        "ping" => {
            note(ctx, "served method=ping");
            Some(rpc_result(id, Value::Object(Vec::new())))
        }
        _ if !*initialized => {
            note(
                ctx,
                &format!("error method={method} code={INVALID_REQUEST}"),
            );
            Some(rpc_error(
                id,
                INVALID_REQUEST,
                "not initialized: send initialize first",
            ))
        }
        "tools/list" => {
            note(ctx, "served method=tools/list");
            Some(rpc_result(id, Value::object([("tools", tools())])))
        }
        "tools/call" => Some(tool_call(ctx, id, message.get("params"))),
        _ => {
            note(
                ctx,
                &format!("error method={method} code={METHOD_NOT_FOUND}"),
            );
            Some(rpc_error(id, METHOD_NOT_FOUND, "method not found"))
        }
    }
}

/// `initialize` — the handshake, and the only place a version is negotiated.
///
/// The rule is the 2025-11-25 lifecycle's: a version this server speaks is
/// echoed back, and everything else — an unknown revision, a missing field, a
/// non-string — is answered with the newest. Never an error: what to do with a
/// version it did not ask for is the client's decision, not the server's.
fn initialize(ctx: &Context, id: Value, params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let protocol = match requested {
        Some(version) if PROTOCOL_VERSIONS.contains(&version) => version,
        _ => PROTOCOL_VERSIONS[0],
    };
    // Who is on the other end, for the day a client's own behaviour is the
    // thing being explained. A name and a version, never the capabilities it
    // claims: those are its business.
    let client = params.and_then(|params| params.get("clientInfo"));
    let named = |key: &str| {
        client
            .and_then(|client| client.get(key))
            .and_then(Value::as_str)
            .unwrap_or(NONE)
    };
    note(
        ctx,
        &format!(
            "served method=initialize client={}/{} requested={} protocol={protocol}",
            named("name"),
            named("version"),
            requested.unwrap_or(NONE)
        ),
    );

    rpc_result(
        id,
        Value::object([
            ("protocolVersion", Value::string(protocol)),
            (
                "capabilities",
                Value::object([("tools", Value::Object(Vec::new()))]),
            ),
            (
                "serverInfo",
                Value::object([
                    ("name", Value::string(SERVER_NAME)),
                    ("version", Value::string(VERSION)),
                ]),
            ),
        ]),
    )
}

/// `tools/call` — the one method that reaches a handler.
///
/// The two failures land in different places on purpose: a call the caller got
/// wrong is a JSON-RPC error, and a verb that ran and refused is a result with
/// `isError` set, because the model is meant to read that one and recover.
fn tool_call(ctx: &Context, id: Value, params: Option<&Value>) -> Value {
    let Some(name) = params
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
    else {
        note(
            ctx,
            &format!("error method=tools/call code={INVALID_PARAMS}"),
        );
        return rpc_error(id, INVALID_PARAMS, "name is required and must be a string");
    };
    // An absent `arguments` is `Null`, which every handler reads as no
    // arguments at all — the same thing an empty object means.
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .cloned()
        .unwrap_or(Value::Null);

    let started = Instant::now();
    match call(ctx, name, &arguments) {
        Ok(answer) => {
            note(
                ctx,
                &format!(
                    "served method=tools/call tool={name} {} ms={}",
                    answer.note,
                    started.elapsed().as_millis()
                ),
            );
            rpc_result(id, content(&answer.result.render(), false))
        }
        Err(McpError::InvalidParams(message)) => {
            note(
                ctx,
                &format!("error method=tools/call code={INVALID_PARAMS}"),
            );
            rpc_error(id, INVALID_PARAMS, &message)
        }
        // The kind alone, never the message: a refusal names the value it
        // refused, and a body that failed to commit must not land in the log
        // instead of the bank.
        Err(McpError::Verb(error)) => {
            note(
                ctx,
                &format!(
                    "failed method=tools/call tool={name} kind={}",
                    error_kind(&error)
                ),
            );
            rpc_result(id, content(&error.to_string(), true))
        }
    }
}

/// A tool result: one text block, and whether it is a refusal.
fn content(text: &str, is_error: bool) -> Value {
    Value::object([
        (
            "content",
            Value::array([Value::object([
                ("type", Value::string("text")),
                ("text", Value::string(text)),
            ])]),
        ),
        ("isError", Value::Bool(is_error)),
    ])
}

/// A JSON-RPC error response. The id is echoed exactly as it arrived — an
/// integer stays an integer, a string a string — because that is the only
/// handle the client has to match the answer to its question.
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    Value::object([
        ("jsonrpc", Value::string(JSONRPC_VERSION)),
        ("id", id),
        (
            "error",
            Value::object([
                ("code", Value::Number(f64::from(code))),
                ("message", Value::string(message)),
            ]),
        ),
    ])
}

/// A JSON-RPC success response.
fn rpc_result(id: Value, result: Value) -> Value {
    Value::object([
        ("jsonrpc", Value::string(JSONRPC_VERSION)),
        ("id", id),
        ("result", result),
    ])
}

/// Write one response as one line, and flush it.
fn write_line(output: &mut impl Write, response: &Value) -> io::Result<()> {
    let mut line = response.render();
    line.push('\n');
    output.write_all(line.as_bytes())?;
    output.flush()
}

/// Journal one line under this verb.
fn note(ctx: &Context, line: &str) {
    crate::journal::note(&ctx.data_root, JOURNAL_VERB, line);
}

/// A verb failure's variant name — what the journal records instead of the
/// message, which can quote the value that was rejected.
fn error_kind(error: &Error) -> &'static str {
    match error {
        Error::Clock => "Clock",
        Error::CrossDevice { .. } => "CrossDevice",
        Error::InvalidInput { .. } => "InvalidInput",
        Error::Io { .. } => "Io",
        Error::Json { .. } => "Json",
        Error::LockHeld { .. } => "LockHeld",
        Error::MissingEnv { .. } => "MissingEnv",
        Error::MissingField { .. } => "MissingField",
        Error::NotFound { .. } => "NotFound",
        Error::Parse { .. } => "Parse",
        Error::Refused { .. } => "Refused",
        Error::ReplacesMissing { .. } => "ReplacesMissing",
        Error::TooManyCollisions { .. } => "TooManyCollisions",
    }
}

/// The `tools/list` array: `recall`, `remember`, `banks`, in that order.
///
/// The order is the contract's, and it is the order of use — a caller reads
/// before it writes, and looks up a key only when it needs one.
pub(crate) fn tools() -> Value {
    Value::array([
        Value::object([
            ("name", Value::string("recall")),
            ("description", Value::string(RECALL_DESCRIPTION)),
            ("inputSchema", recall_schema()),
        ]),
        Value::object([
            ("name", Value::string("remember")),
            ("description", Value::string(REMEMBER_DESCRIPTION)),
            ("inputSchema", remember_schema()),
        ]),
        Value::object([
            ("name", Value::string("banks")),
            ("description", Value::string(BANKS_DESCRIPTION)),
            ("inputSchema", banks_schema()),
        ]),
    ])
}

/// Dispatch one `tools/call` by name.
///
/// `arguments` is the call's own `arguments` object; an absent one arrives as
/// `Value::Null`, which every handler reads as no arguments at all.
pub(crate) fn call(ctx: &Context, name: &str, arguments: &Value) -> Result<Answer, McpError> {
    match name {
        "banks" => banks(ctx, arguments),
        "recall" => recall(ctx, arguments),
        "remember" => remember(ctx, arguments),
        other => Err(McpError::InvalidParams(format!("unknown tool `{other}`"))),
    }
}

/// `recall` — what past sessions know for a working directory.
///
/// The composition is `verbs::recall::compose` and nothing else: this returns
/// its text verbatim alongside the `Recalled` shape, so a caller can see what
/// the budget cut without a second renderer growing downstream.
pub(crate) fn recall(ctx: &Context, params: &Value) -> Result<Answer, McpError> {
    check_arguments(params)?;
    let cwd = optional_str(params, "cwd")?.map_or_else(|| ctx.home.clone(), PathBuf::from);
    let composed =
        crate::verbs::recall::compose(&ctx.data_root, &ctx.home, &cwd, Timestamp::now()?);

    let banks = Value::array(composed.banks.iter().map(|bank| {
        Value::object([
            ("key", Value::string(bank.key.clone())),
            ("degraded", Value::Bool(bank.degraded)),
            (
                "memories",
                Value::array(
                    bank.memories
                        .iter()
                        .map(|file| Value::string(file.as_str())),
                ),
            ),
        ])
    }));
    let trimmed = Value::object([
        (
            "banks_degraded",
            Value::count(composed.trimmed.banks_degraded),
        ),
        (
            "ceiling_lines",
            Value::count(composed.trimmed.ceiling_lines),
        ),
        (
            "sections",
            Value::array(
                composed
                    .trimmed
                    .sections
                    .iter()
                    .map(|name| Value::string(*name)),
            ),
        ),
    ]);

    // The shape of the priming, never its content — the same line `sandman
    // recall` writes, for the same reason: a journal that quoted the memories
    // back would be a second copy of the banks.
    let sections = if composed.trimmed.sections.is_empty() {
        EMPTY.to_owned()
    } else {
        composed.trimmed.sections.join(",")
    };
    let note = format!(
        "cwd={} banks={} memories={} pointers={} chars={} budget={BUDGET_CHARS} \
trimmed={sections} banks_degraded={} cut_lines={}",
        cwd.display(),
        composed.banks.len(),
        composed.memories,
        composed.pointers,
        composed.text.chars().count(),
        composed.trimmed.banks_degraded,
        composed.trimmed.ceiling_lines,
    );

    Ok(Answer {
        note,
        result: Value::object([
            ("text", Value::string(composed.text)),
            ("banks", banks),
            ("memories", Value::count(composed.memories)),
            ("pointers", Value::count(composed.pointers)),
            ("trimmed", trimmed),
        ]),
    })
}

/// `remember` — commit one memory, through the commit path.
///
/// Every default is `verbs::remember`'s own: a `None` here is the CLI's
/// behaviour, not a second set of defaults. An empty body is deliberately not
/// an invalid-params error — the commit path is what refuses it, so the caller
/// gets the same refusal from every door.
pub(crate) fn remember(ctx: &Context, params: &Value) -> Result<Answer, McpError> {
    check_arguments(params)?;
    let Some(Value::String(body)) = params.get("body") else {
        return Err(McpError::InvalidParams(
            "body is required and must be a string".to_owned(),
        ));
    };
    let bank = optional_str(params, "bank")?;
    let cwd = optional_str(params, "cwd")?;
    if bank.is_some() && cwd.is_some() {
        return Err(McpError::InvalidParams(
            "bank and cwd are exclusive".to_owned(),
        ));
    }
    let kind = match optional_str(params, "type")? {
        Some(text) => Some(MemoryType::from_str(text).map_err(|_| {
            McpError::InvalidParams(format!("unknown type `{text}`, expected one of {TYPES}"))
        })?),
        None => None,
    };
    // The server's process cwd is the launcher's and means nothing here, so
    // "neither given" is the home bank — the same bank recall defaults to.
    let cwd = match (bank.is_some(), cwd) {
        (_, Some(cwd)) => Some(PathBuf::from(cwd)),
        (true, None) => None,
        (false, None) => Some(ctx.home.clone()),
    };

    let outcome = crate::verbs::remember::remember(
        &ctx.data_root,
        Remember {
            bank: bank.map(ToOwned::to_owned),
            body: body.clone(),
            cwd,
            description: optional_str(params, "description")?.map(ToOwned::to_owned),
            kind,
            name: optional_str(params, "name")?.map(ToOwned::to_owned),
            session_id: ctx.session_id.clone(),
        },
    )?;

    // Read the bank and the type back off what the commit path wrote rather
    // than restating its defaults. The fallbacks are unreachable — the path is
    // always `<root>/memories/<key>/<file>` — and are here so a hostile shape
    // is a poor journal line instead of a panic.
    let key = outcome
        .path
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .unwrap_or(NONE);
    let kind = outcome.filename.split('_').next().unwrap_or(NONE);
    let result = if outcome.archived.is_some() {
        "replaced"
    } else if outcome.collided {
        "collided"
    } else {
        "created"
    };
    let index = Bank::in_data_root(&ctx.data_root, key).index_path();

    Ok(Answer {
        // A filename is a slug of the name, which the CLI's own remember line
        // already writes; the body, the name and the description are not here
        // and must never be.
        note: format!(
            "bank={key} type={kind} outcome={result} file={}",
            outcome.filename
        ),
        result: Value::object([
            ("bank", Value::string(key)),
            ("file", Value::string(outcome.filename.clone())),
            ("outcome", Value::string(result)),
            ("index", Value::string(index.display().to_string())),
        ]),
    })
}

/// `banks` — every bank under `<root>/memories/`, sorted by key.
///
/// Takes no arguments and ignores whatever it is given: a caller that guesses
/// at a parameter here should get the listing, not an error.
pub(crate) fn banks(ctx: &Context, _params: &Value) -> Result<Answer, McpError> {
    let mut listed = Vec::new();
    let mut live_count = 0_usize;
    for key in bank_keys(&ctx.data_root) {
        let live = is_live(&key);
        if live {
            live_count += 1;
        }
        // An unreadable bank counts zero rather than failing the listing: the
        // key is still the answer to "which banks exist".
        let memories = Bank::in_data_root(&ctx.data_root, &key)
            .memory_filenames()
            .map_or(0, |names| names.len());
        listed.push(Value::object([
            ("key", Value::string(key)),
            ("live", Value::Bool(live)),
            ("memories", Value::count(memories)),
        ]));
    }

    Ok(Answer {
        note: format!("banks={} live={live_count}", listed.len()),
        result: Value::object([("banks", Value::array(listed))]),
    })
}

/// Every bank key under `<root>/memories/`, sorted.
///
/// A bank is a directory; `.recent` and any other dot-directory is not one,
/// and neither is `TOOLS.md` or any other file that shares the parent. A
/// missing `memories/` is an empty list, not a failure — a fresh root has no
/// banks yet and that is not an error to report.
fn bank_keys(data_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(data_root.join(MEMORIES_DIR_NAME)) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
        .filter(|name| !name.starts_with('.'))
        .collect();
    keys.sort();
    keys
}

/// Whether the directory a bank key encodes still exists on this machine.
///
/// `Bank::key_for` is lossy: every non-alphanumeric **byte** became `-`, so a
/// `-` in a key may have been `/`, `.`, `_`, `-`, a space, or one byte of a
/// multi-byte character. `-Users-you--code-project` cannot be read back into a
/// path by splitting on `-`; there is no rule that recovers it, only a search.
///
/// So the decode is a walk. Starting at the root the leading `-` stands for,
/// each directory is listed and each of its subdirectories re-encoded with the
/// same rule; a subdirectory whose encoding is a prefix of what is left of the
/// key — ending the key or followed by the `-` that stood for the separator —
/// is a candidate, and the walk recurses into it. Several candidates can match
/// at one level (`a-b` and `a.b` encode alike); the first that reaches the end
/// of the key answers. Depth is bounded by [`MAX_KEY_COMPONENTS`] so a symlink
/// cycle terminates, and every io error reads as "not live" — an unreadable
/// directory is not evidence that the bank's directory is there.
fn is_live(key: &str) -> bool {
    // The whole encoding starts at `/`: a key that does not is not a path this
    // machine ever produced.
    let Some(rest) = key.strip_prefix('-') else {
        return false;
    };
    descend(Path::new("/"), rest, 0)
}

/// One level of [`is_live`]'s walk: `rest` is the key with everything already
/// matched, and its separator, stripped.
fn descend(dir: &Path, rest: &str, depth: usize) -> bool {
    if rest.is_empty() {
        return true;
    }
    if depth >= MAX_KEY_COMPONENTS {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let encoded = Bank::key_for(Path::new(&name));
        let Some(tail) = rest.strip_prefix(&encoded) else {
            continue;
        };
        // A prefix that neither ends the key nor is followed by the separator
        // matched half a component, which is no match at all.
        if tail.is_empty() {
            return true;
        }
        if let Some(next) = tail.strip_prefix('-')
            && descend(&path, next, depth + 1)
        {
            return true;
        }
    }
    false
}

/// The call's arguments must be an object, or absent.
///
/// A client that sends an array or a number has a bug the model should be told
/// about rather than have read as "no arguments".
fn check_arguments(params: &Value) -> Result<(), McpError> {
    if matches!(params, Value::Null) || params.as_object().is_some() {
        Ok(())
    } else {
        Err(McpError::InvalidParams(
            "arguments must be an object".to_owned(),
        ))
    }
}

/// One optional string argument.
///
/// A missing key and an explicit `null` both mean "not given"; anything else
/// that is not a string names itself in the error, because a schema violation
/// the model cannot locate is a schema violation it will repeat.
fn optional_str<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, McpError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(McpError::InvalidParams(format!("{key} must be a string"))),
    }
}

/// One `{"type": …, "description": …}` schema property — the shape every
/// string parameter takes.
fn property(kind: &str, description: &str) -> Value {
    Value::object([
        ("type", Value::string(kind)),
        ("description", Value::string(description)),
    ])
}

/// `banks`' schema: no parameters at all.
fn banks_schema() -> Value {
    Value::object([
        ("type", Value::string("object")),
        ("properties", Value::Object(Vec::new())),
    ])
}

/// `recall`'s schema.
fn recall_schema() -> Value {
    Value::object([
        ("type", Value::string("object")),
        (
            "properties",
            Value::object([(
                "cwd",
                property(
                    "string",
                    concat!(
                        "The working directory to recall for; its bank and its ",
                        "ancestors' banks answer. Default: the home directory.",
                    ),
                ),
            )]),
        ),
    ])
}

/// `remember`'s schema — `body` required, the rest defaulted by the verb.
fn remember_schema() -> Value {
    Value::object([
        ("type", Value::string("object")),
        ("required", Value::array([Value::string("body")])),
        (
            "properties",
            Value::object([
                (
                    "body",
                    property(
                        "string",
                        "The memory, in markdown. Short: a rule and its reason, not a log.",
                    ),
                ),
                (
                    "bank",
                    property(
                        "string",
                        "The bank key to commit into (from the banks tool). Exclusive with cwd.",
                    ),
                ),
                (
                    "cwd",
                    property(
                        "string",
                        concat!(
                            "The working directory whose bank receives the ",
                            "memory. Exclusive with bank.",
                        ),
                    ),
                ),
                (
                    "type",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::array([
                                Value::string("user"),
                                Value::string("feedback"),
                                Value::string("project"),
                                Value::string("reference"),
                            ]),
                        ),
                        ("description", Value::string("Default: feedback.")),
                    ]),
                ),
                (
                    "name",
                    property(
                        "string",
                        "A short name; default: the body's first eight words.",
                    ),
                ),
                (
                    "description",
                    property("string", "One line; default: the body's first line."),
                ),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        Context, McpError, PROTOCOL_VERSIONS, banks, call, is_live, recall, remember, serve, tools,
    };
    use crate::bank::Bank;
    use crate::error::Error;
    use crate::json::Value;
    use crate::memory::MemoryFile;
    use crate::paths;
    use crate::testutil::TempDir;
    use crate::time::Timestamp;
    use crate::verbs;
    use std::fs;
    use std::io::{self, Cursor, Write};
    use std::path::PathBuf;

    /// The body every seeded memory carries — one sentence, no secrets.
    const SEED_BODY: &str = "the commit path is the only writer of a bank";

    /// A data root with two banks: one keyed for a directory that exists (the
    /// temp dir itself) and one keyed for a directory that never did.
    ///
    /// Seeded through `verbs::remember` so the format authority writes the
    /// files — a test that hand-rolled the format would be testing itself.
    fn seeded(label: &str) -> (TempDir, Context) {
        let temp = TempDir::new(label);
        let data_root = temp.path().join("root");
        let home = temp.path().join("home");
        fs::create_dir_all(&home).expect("home dir");
        verbs::remember::remember(
            &data_root,
            verbs::remember::Remember {
                body: SEED_BODY.to_owned(),
                cwd: Some(temp.path().to_path_buf()),
                ..verbs::remember::Remember::default()
            },
        )
        .expect("seed the live bank");
        verbs::remember::remember(
            &data_root,
            verbs::remember::Remember {
                bank: Some("-nonexistent-place-xyz".to_owned()),
                body: SEED_BODY.to_owned(),
                ..verbs::remember::Remember::default()
            },
        )
        .expect("seed the dead bank");
        (
            temp,
            Context {
                data_root,
                home,
                session_id: None,
            },
        )
    }

    /// A stream that is not there: every write is the client having hung up,
    /// with the kind the caller wants to see handled.
    struct Gone(io::ErrorKind);

    impl Write for Gone {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "the client is not there"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Drive [`serve`] over one scripted stdin — bytes, so a line that is not
    /// UTF-8 can be scripted too — and hand back the response lines.
    fn conversation(ctx: &Context, script: &[u8]) -> Vec<String> {
        let mut answered = Vec::new();
        serve(ctx, Cursor::new(script.to_vec()), &mut answered).expect("serve");
        String::from_utf8(answered)
            .expect("responses are utf-8")
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }

    #[test]
    fn the_transport_answers_bad_lines_and_refuses_work_before_the_handshake() {
        let (_temp, ctx) = seeded("mcp-transport");
        // A blank line, a line that is not JSON, a line that is not UTF-8, a
        // batch array, the wrong protocol version, no method at all, a request
        // too early, a notification nobody handles, a ping, the handshake, and
        // then the same request that was too early a moment ago.
        let mut script: Vec<u8> = Vec::new();
        script.extend_from_slice(b"\n");
        script.extend_from_slice(b"{not json\n");
        script.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"m\":\"\xff\"}\n");
        script.extend_from_slice(br#"[{"jsonrpc":"2.0","id":2,"method":"ping"}]"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"1.0","id":3,"method":"ping"}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","id":4}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","id":5,"method":"tools/list"}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","id":"p","method":"ping"}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","id":0,"method":"initialize"}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","id":6,"method":"tools/list"}"#);
        script.extend_from_slice(b"\n");
        script.extend_from_slice(br#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#);
        script.extend_from_slice(b"\n");

        let answered = conversation(&ctx, &script);
        assert_eq!(answered.len(), 10, "{answered:#?}");
        // Neither the blank line nor the notification is a message to answer;
        // the two unreadable lines are one parse error each, with a null id
        // because there is no id to be read out of them.
        let parse_error =
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#;
        assert_eq!(answered[0], parse_error);
        assert_eq!(answered[1], parse_error);
        // A batch carries no id either; the malformed requests carry theirs.
        assert_eq!(
            answered[2],
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"invalid request"}}"#
        );
        assert!(answered[3].contains(r#""id":3"#), "{}", answered[3]);
        assert!(answered[3].contains(r#""code":-32600"#), "{}", answered[3]);
        assert!(answered[4].contains(r#""id":4"#), "{}", answered[4]);
        assert!(answered[4].contains(r#""code":-32600"#), "{}", answered[4]);
        // Work before the handshake is refused by name, so the client is told
        // what to send rather than left guessing which method was wrong.
        assert!(answered[5].contains(r#""id":5"#), "{}", answered[5]);
        assert!(
            answered[5].contains("not initialized: send initialize first"),
            "{}",
            answered[5]
        );
        // A ping is answered before the handshake: it asks only whether the
        // server is there.
        assert_eq!(answered[6], r#"{"jsonrpc":"2.0","id":"p","result":{}}"#);
        // A handshake with no params at all still negotiates.
        assert!(
            answered[7].contains(&format!(r#""protocolVersion":"{}""#, PROTOCOL_VERSIONS[0])),
            "{}",
            answered[7]
        );
        // …and the request that was too early now answers.
        assert!(
            answered[8].contains(r#""name":"recall""#),
            "{}",
            answered[8]
        );
        assert!(answered[9].contains(r#""code":-32601"#), "{}", answered[9]);

        let journal = fs::read_to_string(paths::run_log(
            &ctx.data_root,
            "mcp",
            Timestamp::now().expect("now"),
        ))
        .expect("the journal");
        assert!(journal.contains("served start"), "{journal}");
        assert!(
            journal.contains("ignored method=notifications/cancelled"),
            "{journal}"
        );
        assert!(
            journal.contains("client=none/none requested=none"),
            "{journal}"
        );
        assert!(journal.contains("served eof"), "{journal}");
    }

    #[test]
    fn initialize_echoes_a_version_it_speaks_and_answers_anything_else_with_the_newest() {
        let (_temp, ctx) = seeded("mcp-negotiate");
        for requested in PROTOCOL_VERSIONS {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":"{requested}"}}}}"#
            );
            let answered = conversation(&ctx, format!("{line}\n").as_bytes());
            assert!(
                answered[0].contains(&format!(r#""protocolVersion":"{requested}""#)),
                "{}",
                answered[0]
            );
        }
        // A revision this server does not speak, a version that is not a
        // string, and no version at all: the newest, never an error.
        for params in [
            r#"{"protocolVersion":"2026-07-28"}"#,
            r#"{"protocolVersion":7}"#,
            r#"{"protocolVersion":null}"#,
            "{}",
        ] {
            let line =
                format!(r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{params}}}"#);
            let answered = conversation(&ctx, format!("{line}\n").as_bytes());
            assert!(
                answered[0].contains(&format!(r#""protocolVersion":"{}""#, PROTOCOL_VERSIONS[0])),
                "{params}: {}",
                answered[0]
            );
            assert!(!answered[0].contains(r#""error""#), "{}", answered[0]);
        }
        // A second handshake is answered like the first: a client that had to
        // ask twice must not find a server that has decided it already knows.
        let handshake = r#"{"jsonrpc":"2.0","id":0,"method":"initialize"}"#;
        let answered = conversation(&ctx, format!("{handshake}\n{handshake}\n").as_bytes());
        assert_eq!(answered.len(), 2);
        assert_eq!(answered[0], answered[1]);
        // An `initialize` sent as a notification is a client bug, not a
        // handshake: it is ignored, and it does not open the server.
        let answered = conversation(
            &ctx,
            concat!(
                r#"{"jsonrpc":"2.0","method":"initialize"}"#,
                "\n",
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                "\n",
            )
            .as_bytes(),
        );
        assert_eq!(answered.len(), 1);
        assert!(answered[0].contains("not initialized"), "{}", answered[0]);
    }

    #[test]
    fn a_client_that_hung_up_is_an_ending_and_any_other_write_failure_is_not() {
        let (_temp, ctx) = seeded("mcp-hangup");
        let line = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let mut script = line.to_vec();
        script.push(b'\n');

        serve(
            &ctx,
            Cursor::new(script.clone()),
            Gone(io::ErrorKind::BrokenPipe),
        )
        .expect("a client that went away is a normal ending");
        let refused = serve(
            &ctx,
            Cursor::new(script),
            Gone(io::ErrorKind::PermissionDenied),
        )
        .expect_err("a write that failed for any other reason is a failure");
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn the_transport_reaches_the_handlers_and_reports_their_two_failures_apart() {
        let (temp, ctx) = seeded("mcp-tools");
        let script = format!(
            concat!(
                r#"{{"jsonrpc":"2.0","id":0,"method":"initialize"}}"#,
                "\n",
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"banks"}}}}"#,
                "\n",
                r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"remember","#,
                r#""arguments":{{"body":"the transport reaches the commit path","cwd":"{cwd}"}}}}}}"#,
                "\n",
                r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"remember","#,
                r#""arguments":{{"body":"  "}}}}}}"#,
                "\n",
                r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"nope"}}}}"#,
                "\n",
                r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{}}}}"#,
                "\n",
            ),
            cwd = temp.path().join("elsewhere").display()
        );

        let answered = conversation(&ctx, script.as_bytes());
        assert_eq!(answered.len(), 6, "{answered:#?}");
        assert!(
            answered[1].contains(r#""isError":false"#),
            "{}",
            answered[1]
        );
        // The tool's own output shape rides inside the text block, escaped.
        assert!(
            answered[2].contains(r#"\"outcome\":\"created\""#),
            "{}",
            answered[2]
        );
        // A verb that refused is a result the model can read; a call the caller
        // got wrong is a protocol error it cannot.
        assert!(answered[3].contains(r#""isError":true"#), "{}", answered[3]);
        assert!(!answered[3].contains(r#""error""#), "{}", answered[3]);
        for refused in &answered[4..] {
            assert!(refused.contains(r#""code":-32602"#), "{refused}");
        }

        let journal = fs::read_to_string(paths::run_log(
            &ctx.data_root,
            "mcp",
            Timestamp::now().expect("now"),
        ))
        .expect("the journal");
        assert!(
            journal.contains("served method=tools/call tool=banks banks=2 live=1 ms="),
            "{journal}"
        );
        assert!(
            journal.contains("failed method=tools/call tool=remember kind=InvalidInput"),
            "{journal}"
        );
        // The shape of a call, never its content.
        assert!(
            !journal.contains("the transport reaches the commit path"),
            "{journal}"
        );
    }

    #[test]
    fn recall_answers_exactly_what_compose_composed() {
        let (temp, ctx) = seeded("mcp-recall");
        let cwd = temp.path();
        let answer = recall(
            &ctx,
            &Value::object([("cwd", Value::string(cwd.to_string_lossy()))]),
        )
        .expect("recall");
        let composed = verbs::recall::compose(
            &ctx.data_root,
            &ctx.home,
            cwd,
            Timestamp::now().expect("now"),
        );

        assert!(!composed.text.is_empty(), "the seeded bank must answer");
        let expected = Value::object([
            ("text", Value::string(composed.text.clone())),
            (
                "banks",
                Value::array(composed.banks.iter().map(|bank| {
                    Value::object([
                        ("key", Value::string(bank.key.clone())),
                        ("degraded", Value::Bool(bank.degraded)),
                        (
                            "memories",
                            Value::array(
                                bank.memories
                                    .iter()
                                    .map(|file| Value::string(file.as_str())),
                            ),
                        ),
                    ])
                })),
            ),
            ("memories", Value::count(composed.memories)),
            ("pointers", Value::count(composed.pointers)),
            (
                "trimmed",
                Value::object([
                    (
                        "banks_degraded",
                        Value::count(composed.trimmed.banks_degraded),
                    ),
                    (
                        "ceiling_lines",
                        Value::count(composed.trimmed.ceiling_lines),
                    ),
                    (
                        "sections",
                        Value::array(
                            composed
                                .trimmed
                                .sections
                                .iter()
                                .map(|name| Value::string(*name)),
                        ),
                    ),
                ]),
            ),
        ]);
        assert_eq!(answer.result, expected);
        // The shape, and never a character of the payload.
        assert!(answer.note.contains(&format!("cwd={}", cwd.display())));
        assert!(answer.note.contains(" budget=9000 "), "{}", answer.note);
        assert!(!answer.note.contains(SEED_BODY), "{}", answer.note);

        // No cwd at all recalls for the home directory.
        let defaulted = recall(&ctx, &Value::Null).expect("recall");
        let at_home = verbs::recall::compose(
            &ctx.data_root,
            &ctx.home,
            &ctx.home,
            Timestamp::now().expect("now"),
        );
        assert_eq!(
            defaulted.result.get("text").and_then(Value::as_str),
            Some(at_home.text.as_str())
        );
    }

    #[test]
    fn remember_writes_the_file_the_cli_would_and_then_collides() {
        let (_temp, mut ctx) = seeded("mcp-remember");
        ctx.session_id = Some("sid-mcp".to_owned());
        let params = Value::object([
            (
                "body",
                Value::string("the queue is the recall surface, not a side effect"),
            ),
            ("cwd", Value::string("/Users/you/code")),
        ]);
        let bank_dir = ctx.data_root.join("memories").join("-Users-you-code");
        let index = bank_dir.join("MEMORY.md");
        let filename = "feedback_the_queue_is_the_recall_surface_not_a.md";

        let answer = remember(&ctx, &params).expect("remember");
        assert_eq!(
            answer.result,
            Value::object([
                ("bank", Value::string("-Users-you-code")),
                ("file", Value::string(filename)),
                ("outcome", Value::string("created")),
                ("index", Value::string(index.display().to_string())),
            ])
        );
        assert_eq!(
            answer.note,
            format!("bank=-Users-you-code type=feedback outcome=created file={filename}")
        );

        let committed = fs::read_to_string(bank_dir.join(filename)).expect("the committed memory");
        assert!(committed.contains("type: feedback\n"), "{committed}");
        assert!(committed.ends_with("the queue is the recall surface, not a side effect\n"));
        let parsed = MemoryFile::parse(&committed).expect("well-formed");
        assert!(
            parsed
                .frontmatter
                .get("source")
                .expect("source")
                .ends_with(" · sid-mcp"),
            "{committed}"
        );
        let regenerated = fs::read_to_string(&index).expect("the index");
        assert!(
            regenerated.contains(&format!("]({filename}) — ")),
            "{regenerated}"
        );

        // The same memory again cannot take the same filename.
        let again = remember(&ctx, &params).expect("remember");
        assert_eq!(
            again.result.get("outcome").and_then(Value::as_str),
            Some("collided")
        );
        assert_eq!(
            again.result.get("file").and_then(Value::as_str),
            Some("feedback_the_queue_is_the_recall_surface_not_a_2.md")
        );
    }

    #[test]
    fn a_bad_call_is_invalid_params_and_a_refused_body_is_the_verbs() {
        let (_temp, ctx) = seeded("mcp-params");
        let body = ("body", Value::string("a body"));

        // Exactly one of bank/cwd resolves the bank.
        assert!(matches!(
            remember(
                &ctx,
                &Value::object([
                    body.clone(),
                    ("bank", Value::string("-a-bank")),
                    ("cwd", Value::string("/Users/you/code")),
                ])
            ),
            Err(McpError::InvalidParams(_))
        ));
        // A type outside the four.
        assert!(matches!(
            remember(
                &ctx,
                &Value::object([body.clone(), ("type", Value::string("memo"))])
            ),
            Err(McpError::InvalidParams(_))
        ));
        // A body that is not a string, and one that is absent.
        assert!(matches!(
            remember(&ctx, &Value::object([("body", Value::Number(5.0))])),
            Err(McpError::InvalidParams(_))
        ));
        assert!(matches!(
            remember(&ctx, &Value::Null),
            Err(McpError::InvalidParams(_))
        ));
        // A parameter of the wrong JSON type names itself.
        let Err(McpError::InvalidParams(message)) =
            recall(&ctx, &Value::object([("cwd", Value::Number(5.0))]))
        else {
            panic!("a numeric cwd must be refused");
        };
        assert!(message.contains("cwd"), "{message}");
        // Arguments that are not an object at all.
        assert!(matches!(
            recall(&ctx, &Value::array([Value::string("cwd")])),
            Err(McpError::InvalidParams(_))
        ));
        // An unknown tool is the caller's error, not a verb failure.
        assert!(matches!(
            call(&ctx, "forget", &Value::Null),
            Err(McpError::InvalidParams(_))
        ));

        // An empty body is the commit path's refusal, so every door refuses it
        // the same way — and nothing is written.
        assert!(matches!(
            remember(&ctx, &Value::object([("body", Value::string("   \n "))])),
            Err(McpError::Verb(Error::InvalidInput { what: "body", .. }))
        ));
        assert!(
            !ctx.data_root
                .join("memories")
                .join(Bank::key_for(&ctx.home))
                .exists()
        );
    }

    #[test]
    fn banks_lists_the_directories_under_memories_with_their_liveness() {
        let (temp, ctx) = seeded("mcp-banks");
        // Noise the listing must ignore: a dot-directory and a stray file.
        fs::create_dir_all(paths::recent_dir(&ctx.data_root)).expect(".recent");
        fs::write(
            ctx.data_root.join("memories").join("TOOLS.md"),
            "the tool index\n",
        )
        .expect("TOOLS.md");

        let answer = banks(&ctx, &Value::Null).expect("banks");
        let mut expected = vec![
            ("-nonexistent-place-xyz".to_owned(), false),
            (Bank::key_for(temp.path()), true),
        ];
        expected.sort();
        assert_eq!(
            answer.result,
            Value::object([(
                "banks",
                Value::array(expected.into_iter().map(|(key, live)| Value::object([
                    ("key", Value::string(key)),
                    ("live", Value::Bool(live)),
                    ("memories", Value::count(1)),
                ])))
            )])
        );
        assert_eq!(answer.note, "banks=2 live=1");

        // A root with no `memories/` at all lists nothing, and does not fail.
        let empty = Context {
            data_root: temp.path().join("no-root"),
            home: ctx.home.clone(),
            session_id: None,
        };
        assert_eq!(
            banks(&empty, &Value::Null).expect("banks").result,
            Value::object([("banks", Value::Array(Vec::new()))])
        );
    }

    #[test]
    fn the_tool_list_is_recall_remember_banks_with_object_schemas() {
        let listed = tools();
        let entries = listed.as_array().expect("an array of tools");
        let names: Vec<&str> = entries
            .iter()
            .filter_map(|tool| tool.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(names, ["recall", "remember", "banks"]);
        for tool in entries {
            assert!(
                tool.get("description")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty()),
                "{tool:?}"
            );
            assert!(
                tool.get("inputSchema").and_then(Value::as_object).is_some(),
                "{tool:?}"
            );
        }
    }

    #[test]
    fn is_live_decodes_a_key_by_walking_the_filesystem() {
        let temp = TempDir::new("mcp-live");
        // Three components whose separators all encode to the same `-`.
        let nested = temp.path().join("a-b").join("c.d").join("e_f");
        fs::create_dir_all(&nested).expect("the nested path");
        assert!(is_live(&Bank::key_for(&nested)));
        assert!(!is_live(&Bank::key_for(&nested.join("missing"))));
        // The root itself is always live; a key that does not start at it is
        // not a path this machine encoded.
        assert!(is_live("-"));
        assert!(!is_live("Users-you-code"));
        assert!(!is_live(""));
    }

    #[test]
    fn a_dispatched_call_reaches_the_same_handler_as_a_direct_one() {
        let (_temp, ctx) = seeded("mcp-dispatch");
        let listed = call(&ctx, "banks", &Value::Null).expect("banks");
        assert_eq!(
            listed.result,
            banks(&ctx, &Value::Null).expect("banks").result
        );

        let params = Value::object([("body", Value::string("dispatch goes through call"))]);
        let written = call(&ctx, "remember", &params).expect("remember");
        let path = PathBuf::from(
            written
                .result
                .get("index")
                .and_then(Value::as_str)
                .expect("an index path"),
        );
        assert!(path.is_file(), "{}", path.display());
    }
}
