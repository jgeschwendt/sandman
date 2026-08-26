//! Claude Code hook payloads — the two sandman is called with.
//!
//! `SessionEnd` hands `take` a session and why it ended, `SessionStart` hands
//! `recall` a working directory. Both payloads carry fields sandman does not
//! use, and both will grow more: unknown fields are ignored by construction,
//! and a field of the wrong type reads as absent rather than as a failure.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::json::{self, Value};

/// The one `SessionEnd` reason that is not an ending.
///
/// Claude Code fires it on the session it is *adopting*, at the moment it is
/// adopted — `if (adoptedSessionId) sessionEnd(adoptedSessionId, "resume", …)`
/// in 2.1.246. The conversation is beginning, not over, and its transcript has
/// to stay where the resumed turn will append to it.
pub const RESUME_REASON: &str = "resume";

/// What sandman needs out of a `SessionEnd` payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionEnd {
    /// The session's working directory, when the payload names one.
    pub cwd: Option<PathBuf>,
    /// Why the session ended — `clear` · `logout` · `other` ·
    /// `prompt_input_exit` · `resume`. Absent in payloads that predate the
    /// field, which read as an ordinary ending.
    pub reason: Option<String>,
    /// The session to take. Absent means there is nothing to do.
    pub session_id: Option<String>,
}

impl SessionEnd {
    /// Whether this payload is a resume wearing an ending's name.
    #[must_use]
    pub fn is_resume(&self) -> bool {
        self.reason.as_deref() == Some(RESUME_REASON)
    }
}

/// What sandman needs out of a `SessionStart` payload.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionStart {
    /// The session's working directory, when the payload names one.
    pub cwd: Option<PathBuf>,
}

/// Read a `SessionEnd` payload.
pub fn session_end(payload: &str) -> Result<SessionEnd> {
    let value = read(payload)?;
    Ok(SessionEnd {
        cwd: text(&value, "cwd").map(PathBuf::from),
        reason: text(&value, "reason"),
        session_id: text(&value, "session_id"),
    })
}

/// Read a `SessionStart` payload.
pub fn session_start(payload: &str) -> Result<SessionStart> {
    let value = read(payload)?;
    Ok(SessionStart {
        cwd: text(&value, "cwd").map(PathBuf::from),
    })
}

/// The `SessionStart` reply that injects `context` into the new session —
/// the envelope `memory-recall.js` writes today.
#[must_use]
pub fn session_start_reply(context: &str) -> String {
    Value::Object(vec![(
        "hookSpecificOutput".to_owned(),
        Value::Object(vec![
            ("hookEventName".to_owned(), Value::string("SessionStart")),
            ("additionalContext".to_owned(), Value::string(context)),
        ]),
    )])
    .render()
}

/// Parse a payload into an object; anything else is a typed error.
fn read(payload: &str) -> Result<Value> {
    let value = json::parse(payload.trim()).map_err(|source| Error::Json {
        path: None,
        message: format!("hook payload: {source}"),
    })?;
    if matches!(value, Value::Object(_)) {
        Ok(value)
    } else {
        Err(Error::Json {
            path: None,
            message: "hook payload: expected a json object".to_owned(),
        })
    }
}

/// A string field, or `None` when it is absent or not a string.
fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{session_end, session_start, session_start_reply};
    use crate::error::Error;
    use std::path::PathBuf;

    #[test]
    fn a_well_formed_session_end_payload_reads_both_fields() {
        let payload = r#"{"session_id":"aaaabbbb","cwd":"/Users/you/code"}"#;
        let parsed = session_end(payload).expect("parse");
        assert_eq!(parsed.session_id.as_deref(), Some("aaaabbbb"));
        assert_eq!(parsed.cwd, Some(PathBuf::from("/Users/you/code")));
        // A payload from before the field, and every ordinary ending, is not
        // a resume.
        assert_eq!(parsed.reason, None);
        assert!(!parsed.is_resume());
    }

    #[test]
    fn only_the_resume_reason_reads_as_a_beginning() {
        let read = |reason: &str| {
            session_end(&format!(
                r#"{{"session_id":"abc","hook_event_name":"SessionEnd","reason":"{reason}"}}"#
            ))
            .expect("parse")
        };
        for reason in ["clear", "logout", "other", "prompt_input_exit"] {
            let parsed = read(reason);
            assert_eq!(parsed.reason.as_deref(), Some(reason));
            assert!(!parsed.is_resume(), "{reason} is an ending");
        }
        let resumed = read(super::RESUME_REASON);
        assert_eq!(resumed.reason.as_deref(), Some("resume"));
        assert!(resumed.is_resume());
        // A reason of the wrong type reads as absent, never as a resume.
        assert!(
            !session_end(r#"{"session_id":"abc","reason":7}"#)
                .expect("parse")
                .is_resume()
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let payload = concat!(
            r#"{"hook_event_name":"SessionEnd","reason":"clear","session_id":"abc","#,
            r#""transcript_path":"/x.jsonl","cwd":"/tmp","permission_mode":"auto","#,
            r#""nested":{"deep":[1,2,{"deeper":true}]}}"#
        );
        let parsed = session_end(payload).expect("parse");
        assert_eq!(parsed.session_id.as_deref(), Some("abc"));
        assert_eq!(parsed.cwd, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn a_payload_without_a_session_id_is_parsed_not_rejected() {
        for payload in [
            "{}",
            r#"{"cwd":"/tmp"}"#,
            r#"{"session_id":null}"#,
            r#"{"session_id":42}"#,
            r#"{"session_id":""}"#,
        ] {
            let parsed = session_end(payload).expect("parse");
            assert_eq!(parsed.session_id, None, "for {payload}");
        }
    }

    #[test]
    fn a_malformed_payload_is_a_typed_error() {
        for payload in ["", "not json", "[]", r#"{"session_id":}"#] {
            assert!(
                matches!(session_end(payload), Err(Error::Json { .. })),
                "for {payload:?}"
            );
        }
    }

    #[test]
    fn a_session_start_payload_yields_the_cwd() {
        assert_eq!(
            session_start(r#"{"cwd":"/Users/you","source":"startup"}"#)
                .expect("parse")
                .cwd,
            Some(PathBuf::from("/Users/you"))
        );
        assert_eq!(session_start("{}").expect("parse").cwd, None);
    }

    #[test]
    fn the_session_start_reply_carries_the_context_escaped() {
        let reply = session_start_reply("line one\nline \"two\"");
        assert_eq!(
            reply,
            concat!(
                r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","#,
                r#""additionalContext":"line one\nline \"two\""}}"#
            )
        );
    }
}
