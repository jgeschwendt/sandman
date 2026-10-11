//! The report as a page: the night page `report serve` answers `GET /` with,
//! and a markdown renderer covering exactly what reports and proposal bodies
//! use — `#`/`##` headings, `-` bullets, pipe tables, paragraphs, inline
//! `` `code` `` and `**bold**`; anything else passes through as escaped text.
//! No script: plain forms and `<details>`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::{HEADINGS, proposals, section};
use crate::json::Value;
use crate::paths;
use crate::verbs::shift::{self, Local, Meta, Night, WEEK};

/// The answers a proposal's form offers, in the order its buttons show.
pub(crate) const ANSWERS: [&str; 3] = ["approve", "revise", "decline"];
/// How many nights the footer links.
const NAV_NIGHTS: usize = 14;
/// `ui`'s palette and type scale (jgeschwendt/ui `ui.css`): white and green
/// by light, black and gold by dark.
const STYLE: &str = r#"
:root{color-scheme:light dark;--bg:#ffffff;--surface:#fbfcfa;--sunken:#f2f4f0;
--border:#e2e6df;--fg:#141c15;--muted:#5a6b5b;--faint:#879287;--accent:#476349;
--accent-fg:#ffffff;--soft:#eaefe9;
--mono:ui-monospace,SFMono-Regular,"SF Mono",Menlo,Consolas,monospace;
--sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Roboto,"Helvetica Neue",sans-serif}
@media (prefers-color-scheme:dark){:root{--bg:#08090a;--surface:#101113;
--sunken:#0c0d0f;--border:#24262a;--fg:#ece7db;--muted:#9b9487;--faint:#6b675e;
--accent:#cfa14d;--accent-fg:#14100a;--soft:rgba(207,161,77,.14)}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:.875rem/1.55 var(--sans)}
main{margin:0 auto;max-width:46rem;padding:3rem 1rem}
header h1{color:var(--accent);font-size:1.625rem;letter-spacing:-.011em;margin:0}
header p{color:var(--muted);margin:.25rem 0 0}
section>h2{color:var(--faint);font:550 .6875rem var(--mono);letter-spacing:.07em;
margin:2.5rem 0 .75rem;text-transform:uppercase}
h1,h2,h3{line-height:1.25;overflow-wrap:break-word}
article,details{background:var(--surface);border:1px solid var(--border);
border-radius:12px;margin:0 0 .75rem;padding:1rem}
article h2,details h1,details h2{font-size:1rem;margin:1rem 0 .5rem}
article h3{font-size:1rem;margin:0}
summary{cursor:pointer;font-weight:550}
.meta{color:var(--muted);font-size:.75rem;margin:.25rem 0}
a{color:var(--accent)}
code{background:var(--sunken);border-radius:4px;font:.8125rem var(--mono);padding:0 4px}
table{border-collapse:collapse;display:block;font-size:.8125rem;margin:.5rem 0;
max-width:100%;overflow-x:auto}
th,td{border-bottom:1px solid var(--border);padding:4px 8px;text-align:left}
th{color:var(--muted);font-weight:550}
li{margin:.25rem 0}
form{display:flex;flex-wrap:wrap;gap:8px;margin-top:.75rem}
input,button{border:1px solid var(--border);border-radius:8px;font:inherit;
padding:6px 12px}
input{background:var(--bg);color:var(--fg);flex:1 1 12rem}
button{background:var(--soft);color:var(--fg);cursor:pointer}
button[value=approve]{background:var(--accent);border-color:var(--accent);
color:var(--accent-fg)}
footer{border-top:1px solid var(--border);margin-top:3rem;padding-top:1rem}
nav{display:flex;flex-wrap:wrap;font:.75rem var(--mono);gap:4px 12px}
"#;

/// `text` safe in HTML text and in a quoted attribute.
pub(crate) fn escape(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
        out
    })
}

/// `text` split on `mark`: each closed span through `wrap`, the rest — an
/// unclosed mark with it — through `plain`.
fn spans(
    text: &str,
    mark: &str,
    wrap: &dyn Fn(&str) -> String,
    plain: &dyn Fn(&str) -> String,
) -> String {
    let parts: Vec<&str> = text.split(mark).collect();
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| match (index % 2, index + 1 < parts.len()) {
            (1, true) => wrap(part),
            (1, false) => plain(&format!("{mark}{part}")),
            _ => plain(part),
        })
        .collect()
}

/// One line's inline markdown: `` `code` `` and `**bold**`, the rest escaped.
fn inline(text: &str) -> String {
    let bold = |text: &str| {
        spans(
            &escape(text),
            "**",
            &|bold| format!("<strong>{bold}</strong>"),
            &ToOwned::to_owned,
        )
    };
    spans(
        text,
        "`",
        &|code| format!("<code>{}</code>", escape(code)),
        &bold,
    )
}

/// A pipe row's cells, rendered.
fn cells(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| inline(cell.trim()))
        .collect()
}

/// Whether a pipe row is a table's `| --- |` separator.
fn rule(line: &str) -> bool {
    line.trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':')
    })
}

/// A table of rendered cells.
fn table(head: &[String], rows: &[Vec<String>]) -> String {
    let row = |tag: &str, cells: &[String]| {
        let cells = cells
            .iter()
            .map(|cell| format!("<{tag}>{cell}</{tag}>"))
            .collect::<Vec<_>>()
            .concat();
        format!("<tr>{cells}</tr>")
    };
    let body: String = rows.iter().map(|cells| row("td", cells)).collect();
    format!(
        "<table><thead>{}</thead><tbody>{body}</tbody></table>",
        row("th", head)
    )
}

/// A line's block: blank, heading, bullet, pipe row or text.
fn block(line: &str) -> u8 {
    match line {
        "" => 0,
        _ if line.starts_with("# ") || line.starts_with("## ") => 1,
        _ if line.starts_with("- ") => 2,
        _ if line.starts_with('|') => 3,
        _ => 4,
    }
}

/// `text` as HTML.
pub(crate) fn markdown(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut out = String::new();
    let mut rest = &lines[..];
    while let Some(first) = rest.first() {
        let kind = block(first);
        let (run, after) =
            rest.split_at(rest.iter().take_while(|line| block(line) == kind).count());
        rest = after;
        match kind {
            0 => {}
            1 => {
                for line in run {
                    let (tag, title) = line
                        .strip_prefix("## ")
                        .map_or(("h1", &line[2..]), |title| ("h2", title));
                    let _ = write!(out, "<{tag}>{}</{tag}>", inline(title));
                }
            }
            2 => {
                let items = run
                    .iter()
                    .map(|line| format!("<li>{}</li>", inline(&line[2..])))
                    .collect::<Vec<_>>()
                    .concat();
                let _ = write!(out, "<ul>{items}</ul>");
            }
            3 if run.len() > 1 && rule(run[1]) => {
                let rows: Vec<Vec<String>> = run[2..].iter().map(|line| cells(line)).collect();
                out.push_str(&table(&cells(run[0]), &rows));
            }
            _ => {
                let lines: Vec<String> = run.iter().map(|line| inline(line)).collect();
                let _ = write!(out, "<p>{}</p>", lines.join("<br>"));
            }
        }
    }
    out
}

/// One shift of a night.
struct Shift {
    /// Its `meta.json`'s `cost_usd`, what its handoff recorded.
    cost: f64,
    /// Its `meta.json`.
    meta: Meta,
    /// Its `report.md`, empty until written.
    report: String,
}

/// Every started shift by the night it began in, keyed by that night's
/// anchor date — `Night::anchor`, the day the engine took its theme from.
fn nights(data_root: &Path, night: Night) -> BTreeMap<String, (Local, Vec<Shift>)> {
    let mut nights: BTreeMap<String, (Local, Vec<Shift>)> = BTreeMap::new();
    for meta in shift::metas(data_root) {
        let Some(at) = meta.started else { continue };
        let local = night.anchor(at);
        let report = fs::read_to_string(paths::shift_dir(data_root, &meta.shift).join("report.md"));
        let shift = Shift {
            cost: meta
                .value
                .get("cost_usd")
                .and_then(Value::as_number)
                .unwrap_or_default(),
            report: report.unwrap_or_default(),
            meta,
        };
        nights
            .entry(local.ymd())
            .or_insert((local, Vec::new()))
            .1
            .push(shift);
    }
    nights
}

/// `items` inside `<tag>`, or `none.`.
fn list(tag: &str, items: &str) -> String {
    if items.is_empty() {
        "<p>none.</p>".to_owned()
    } else {
        format!("<{tag}>{items}</{tag}>")
    }
}

/// The page for the night anchored on `wanted` (`yyyy-mm-dd`), the newest
/// night with a shift when `None`; `None` when no shift began in that night.
pub(crate) fn page(
    data_root: &Path,
    threads: &Path,
    night: Night,
    wanted: Option<&str>,
) -> Option<String> {
    let nights = nights(data_root, night);
    let chosen = match wanted {
        Some(key) => Some(nights.get(key)?),
        None => nights.values().next_back(),
    };
    let awaiting: Vec<_> = proposals(threads, None)
        .into_iter()
        .filter(|p| p.awaiting)
        .collect();
    let (mut eye, mut details, mut head, mut rows) =
        (String::new(), String::new(), None, Vec::new());
    for shift in chosen.iter().flat_map(|(_, shifts)| shifts.iter().rev()) {
        let id = escape(&shift.meta.shift);
        let link = format!("<a href=\"#{id}\">{id}</a>");
        for line in section(&shift.report, HEADINGS[0]).filter(|line| line.starts_with("- ")) {
            let _ = write!(eye, "<li>{} — {link}</li>", inline(&line[2..]));
        }
        let mut landed = section(&shift.report, HEADINGS[1]).filter(|line| line.starts_with('|'));
        if let Some(first) = landed.next() {
            head.get_or_insert_with(|| [vec!["shift".to_owned()], cells(first)].concat());
        }
        rows.extend(
            landed
                .skip(1)
                .map(|line| [vec![link.clone()], cells(line)].concat()),
        );
        let text = |key| {
            escape(
                shift
                    .meta
                    .value
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("-"),
            )
        };
        let _ = write!(
            details,
            "<details id=\"{id}\"><summary>{id} · {} · {} · ${:.2}</summary>{}</details>",
            text("theme"),
            text("repo"),
            shift.cost,
            markdown(&shift.report)
        );
    }
    let line = chosen.map_or_else(
        || "no nights yet".to_owned(),
        |(local, shifts)| {
            format!(
                "{}→{} night · {} shifts · ${:.2} · {} landed · {} proposals",
                local.day().name,
                WEEK[(local.weekday + 1) % 7].name,
                shifts.len(),
                shifts.iter().map(|shift| shift.cost).sum::<f64>(),
                rows.len(),
                awaiting.len()
            )
        },
    );
    let forms = awaiting
        .iter()
        .map(|p| {
            let body: Vec<&str> = p.body.lines().filter(|line| !line.starts_with("# ")).collect();
            let buttons = ANSWERS
                .iter()
                .map(|answer| format!("<button name=\"answer\" value=\"{answer}\">{answer}</button>"))
                .collect::<Vec<_>>()
            .concat();
            format!(
                "<article><h3>{}</h3><p class=\"meta\">{} · opened {}</p>{}<form method=\"post\" action=\"/answer\"><input type=\"hidden\" name=\"key\" value=\"{}\"><input name=\"note\" placeholder=\"note\" aria-label=\"note\">{buttons}</form></article>",
                escape(&p.title),
                escape(p.key.split(':').nth(1).unwrap_or("-")),
                escape(&p.opened),
                markdown(&body.join("\n")),
                escape(&p.key)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let landed = head
        .filter(|_| !rows.is_empty())
        .map(|head| table(&head, &rows))
        .unwrap_or_default();
    let nav = nights
        .keys()
        .rev()
        .take(NAV_NIGHTS)
        .map(|key| format!("<a href=\"/?night={key}\">{key}</a>"))
        .collect::<Vec<_>>()
        .concat();
    Some(format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>sandman · nights</title><style>{STYLE}</style></head><body><main><header><h1>sandman · nights</h1><p>{}</p></header><section><h2>Proposals awaiting you</h2>{}</section><section><h2>Worth your eye</h2>{}</section><section><h2>Landed</h2>{}</section><section><h2>Shifts</h2>{}</section><footer><nav>{nav}</nav></footer></main></body></html>\n",
        escape(&line),
        list("div", &forms),
        list("ul", &eye),
        list("div", &landed),
        list("div", &details)
    ))
}

#[cfg(test)]
mod tests {
    use super::markdown;

    #[test]
    fn markdown_renders_what_reports_use_and_escapes_the_rest() {
        assert_eq!(
            markdown(
                "# T <i> & \"'\n## H\n\n- `a<b>` **b**\n- c\n| x | y |\n| --- | :-: |\n| **1** | 2 |\n| not | a table |\ntext `open\n### deeper **open\n"
            ),
            "<h1>T &lt;i&gt; &amp; &quot;&#39;</h1><h2>H</h2><ul><li><code>a&lt;b&gt;</code> <strong>b</strong></li><li>c</li></ul><table><thead><tr><th>x</th><th>y</th></tr></thead><tbody><tr><td><strong>1</strong></td><td>2</td></tr><tr><td>not</td><td>a table</td></tr></tbody></table><p>text `open<br>### deeper **open</p>"
        );
        assert_eq!(markdown("| a |\nb"), "<p>| a |</p><p>b</p>");
    }
}
