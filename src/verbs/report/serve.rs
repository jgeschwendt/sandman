//! `report serve` — the report as a loopback page. HTTP/1.1 over a std
//! listener, one request at a time, `Connection: close`: `GET /` is a night's
//! page, `POST /answer` replies to a proposal thread through the `docket`
//! binary and puts the owner's decision on the trail, `POST /rotation` sets
//! which repos the rotation leaves off and puts that on the trail as a guard.
//!
//! The listener is loopback-only, and a request whose `Host` is not the bound
//! address, or whose `Origin` is not its `http://` origin, is refused — so
//! neither a rebinding DNS name nor another site's form reaches the answers.

use std::fmt::Display;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::html::{self, ANSWERS, escape};
use super::{mark_read, proposals};
use crate::time::Timestamp;
use crate::verbs::{shift, trail};

/// The default listen address.
pub const BIND: &str = "127.0.0.1:1705";
/// The largest request body read.
const BODY_BYTES: usize = 64 * 1024;
/// How long a client may take to send its request.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// What `report serve` reads and runs, resolved by the caller.
#[derive(Clone, Debug)]
pub struct Server {
    /// The shift engine's context: the data root, the night that keys the
    /// pages, `~/.docket`, the rotation's roots.
    pub ctx: shift::Context,
    /// The `docket` binary the answers go through.
    pub docket: PathBuf,
    /// Whether `ctx.now` is `$SANDMAN_NOW`'s; else each request reads the clock.
    pub pinned: bool,
    /// How many requests to answer before returning; `None` serves on.
    pub requests: Option<usize>,
}

/// Method, target, headers (names lowercase), body.
type Request = (String, String, Vec<(String, String)>, String);
/// A status line, extra header lines, a body.
type Response = (&'static str, String, String);

/// Print `listening on http://<address>` and answer requests one at a time.
pub fn serve(listener: &TcpListener, server: &Server) {
    // a bound listener always has an address; were it to have none, every
    // request's Host would mismatch the empty one and be refused
    let bound = listener
        .local_addr()
        .map(|address| address.to_string())
        .unwrap_or_default();
    println!("listening on http://{bound}");
    for mut stream in listener
        .incoming()
        .flatten()
        .take(server.requests.unwrap_or(usize::MAX))
    {
        let (status, headers, body) = respond(server, &bound, &stream);
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\n{headers}Content-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

/// A bare page for a status, `detail` below it.
fn status(line: &'static str, detail: &str) -> Response {
    (
        line,
        String::new(),
        format!("<!doctype html>\n<title>{line}</title><p>{line}</p>{detail}\n"),
    )
}

/// One request read off `stream`; `None` when it is malformed, too large or
/// too slow.
fn read(stream: &TcpStream) -> Option<Request> {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let mut reader = BufReader::new(stream);
    // a line that will not read stays empty, and an empty line ends the head;
    // the head is read whole before it is judged, so a refusal never closes
    // on unread bytes — which would reset the connection under the answer
    let mut request = String::new();
    let _ = reader.read_line(&mut request);
    let (mut headers, mut line) = (Vec::new(), String::new());
    while let Some((name, value)) = {
        line.clear();
        let _ = reader.read_line(&mut line);
        line.split_once(':')
    } {
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let mut words = request.split_whitespace().map(ToOwned::to_owned);
    let (method, target) = words.next().zip(words.next())?;
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map_or(Some(0), |(_, length)| length.parse().ok())
        .filter(|length| *length <= BODY_BYTES)?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some((
        method,
        target,
        headers,
        String::from_utf8_lossy(&body).into_owned(),
    ))
}

/// `a=1&b=2`, percent- and plus-decoded.
fn form(text: &str) -> Vec<(String, String)> {
    let decode = |text: &str| {
        let mut bytes = Vec::new();
        let mut rest = text.as_bytes();
        while let [byte, after @ ..] = rest {
            let digit = |at: usize| {
                after
                    .get(at)
                    .and_then(|digit| char::from(*digit).to_digit(16))
            };
            rest = after;
            bytes.push(match (byte, digit(0).zip(digit(1))) {
                (b'%', Some((high, low))) => {
                    rest = &after[2..];
                    u8::try_from(high * 16 + low).unwrap_or_default()
                }
                (b'+', _) => b' ',
                _ => *byte,
            });
        }
        String::from_utf8_lossy(&bytes).into_owned()
    };
    text.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (decode(key), decode(value)))
        .collect()
}

/// The answer to one request.
fn respond(server: &Server, bound: &str, stream: &TcpStream) -> Response {
    let ctx = &shift::Context {
        now: Timestamp::now()
            .ok()
            .filter(|_| !server.pinned)
            .unwrap_or(server.ctx.now),
        ..server.ctx.clone()
    };
    let Some((method, target, headers, body)) = read(stream) else {
        return status("400 Bad Request", "");
    };
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    if header("host") != Some(bound)
        || header("origin").is_some_and(|origin| origin != format!("http://{bound}"))
    {
        return status("403 Forbidden", "");
    }
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    match (method.as_str(), path) {
        ("GET", "/") => {
            let night = form(query).into_iter().find(|(key, _)| key == "night");
            let rotation = match shift::rotation(ctx) {
                Ok(rotation) => rotation,
                Err(error) => return failed(&error),
            };
            html::page(
                &ctx.data_root,
                &ctx.docket,
                ctx.night,
                night.as_ref().map(|(_, night)| night.as_str()),
                &rotation,
            )
            .map_or_else(
                || status("404 Not Found", "<p>no shift began that night</p>"),
                |(page, newest)| {
                    // the owner's eyes on the newest night are the acknowledgment;
                    // an older night's page leaves the cursor; a cursor that will
                    // not write still serves the page
                    if let Some(at) = newest.filter(|_| night.is_none()) {
                        let _ = mark_read(&ctx.data_root, at);
                    }
                    ("200 OK", String::new(), page)
                },
            )
        }
        ("POST", "/answer") => answer(server, ctx, &form(&body)),
        ("POST", "/rotation") => rotation(ctx, &form(&body)),
        ("GET" | "POST", _) => status("404 Not Found", ""),
        _ => (
            "405 Method Not Allowed",
            "Allow: GET, POST\r\n".to_owned(),
            String::new(),
        ),
    }
}

/// Reply to the open proposal thread `key` names with `answer` (and `note`
/// as its body) through the `docket` binary, then append the `decision`.
fn answer(server: &Server, ctx: &shift::Context, fields: &[(String, String)]) -> Response {
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.trim())
    };
    let (key, said, note) = (field("key"), field("answer"), field("note"));
    if !ANSWERS.contains(&said) {
        return status(
            "400 Bad Request",
            "<p>answer approve, revise or decline</p>",
        );
    }
    let Some(thread) = proposals(&ctx.docket, None)
        .into_iter()
        .find(|proposal| proposal.key == key)
    else {
        return status("404 Not Found", "<p>no open proposal has that key</p>");
    };
    let mut reply = Command::new(&server.docket);
    reply
        .args(["reply", &thread.id, "--as", "user", "--answer", said])
        .stdin(Stdio::null());
    if !note.is_empty() {
        reply.args(["--body", note]);
    }
    let failed = match reply.output() {
        Ok(output) if output.status.success() => None,
        Ok(output) => Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        Err(error) => Some(error.to_string()),
    };
    if let Some(why) = failed {
        return status("502 Bad Gateway", &format!("<pre>{}</pre>", escape(&why)));
    }
    seen(trail::append(
        &ctx.data_root,
        trail::Draft {
            kind: "decision".to_owned(),
            rationale: Some(format!("owner {said}d proposal {key}")),
            refs: vec![format!("thread:{}", thread.id)],
            repo: key.split(':').nth(1).map(ToOwned::to_owned),
            ..trail::Draft::default()
        },
    ))
}

/// Leave off every rotation repo the form does not check, then append the
/// owner's `guard`; names outside the rotation are ignored.
fn rotation(ctx: &shift::Context, fields: &[(String, String)]) -> Response {
    let on: Vec<&(String, String)> = fields.iter().filter(|(key, _)| key == "on").collect();
    let off = shift::trunks(ctx)
        .into_iter()
        .map(|(label, _)| label)
        .filter(|label| !on.iter().any(|(_, repo)| repo == label));
    seen(shift::set_off(ctx, off).and_then(|line| {
        trail::append(
            &ctx.data_root,
            trail::Draft {
                kind: "guard".to_owned(),
                rationale: Some(format!("owner set the rotation: {line}")),
                ..trail::Draft::default()
            },
        )
    }))
}

/// `303` back to `/` once a write went through, `500` with why it did not.
fn seen<T, E: Display>(written: Result<T, E>) -> Response {
    written.map_or_else(
        |error| failed(&error),
        |_| ("303 See Other", "Location: /\r\n".to_owned(), String::new()),
    )
}

/// `500` with why.
fn failed(error: &dyn Display) -> Response {
    status(
        "500 Internal Server Error",
        &format!("<pre>{}</pre>", escape(&error.to_string())),
    )
}
