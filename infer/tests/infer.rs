//! End to end: the built `sandman-infer` over temp roots and the tiny GGUFs beside this file
//! (`fixture.py`), every daemon a test starts killed when its guard drops; `parity` holds it to
//! the real models and the official encoder's golden when `SANDMAN_INFER_PARITY` names them.

// the binary's dependency, unused by the suite
use signal_hook as _;

#[cfg(test)]
mod tests {
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::LlamaModel;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::token::LlamaToken;
    use llama_cpp_2::{LogOptions, send_logs_to_tracing};
    use serde_json::{Value, json};
    use std::error::Error;
    use std::fs::{self, File, OpenOptions, Permissions};
    use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
    use std::num::NonZeroU32;
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _, symlink};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio, id};
    use std::time::{Duration, Instant};
    use std::{env, iter, thread};

    /// The decision model's file under the models dir.
    const DECIDER: &str = "decider-flash-v1.2-q8_0.gguf";
    /// The embedding model's file under the models dir.
    const EMBEDDER: &str = "qwen3-embedding-0.6b-q8_0.gguf";
    /// The build identity the binary stamps (`build.rs`).
    const VERSION: &str = concat!(env!("SANDMAN_RELEASE"), "-", env!("SANDMAN_BUILD"));

    /// A test's fallible outcome.
    type Outcome = Result<(), Box<dyn Error>>;

    /// A running `sandman-infer` and its stdin, held apart from the child (whose `wait` would close
    /// it, racing a signal's stop) as `serve` holds it; killed and reaped when it drops.
    struct Daemon(Child, Option<ChildStdin>);

    impl Drop for Daemon {
        fn drop(&mut self) {
            drop(self.0.kill());
            drop(self.0.wait());
        }
    }

    /// `request` to the daemon over `root`, and its whole reply.
    fn ask(root: &str, request: &str) -> io::Result<String> {
        let mut stream = UnixStream::connect(format!("{root}/run/infer.sock"))?;
        stream.write_all(request.as_bytes())?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map(|_| reply)
    }

    /// `request` as one line to the daemon over `root`, its reply parsed.
    fn call(root: &str, request: &Value) -> Result<Value, Box<dyn Error>> {
        Ok(serde_json::from_str(&ask(root, &format!("{request}\n"))?)?)
    }

    /// `kill <signal> <pid>`: whether the signal was delivered.
    fn kill(signal: &str, daemon: &Daemon) -> io::Result<bool> {
        Ok(Command::new("kill")
            .args([signal, &daemon.0.id().to_string()])
            .status()?
            .success())
    }

    /// `root`'s log lines, `at` as `A` and `ms` as 0.
    fn log(root: &str, file: &str) -> Result<Vec<String>, Box<dyn Error>> {
        fs::read_to_string(format!("{root}/run/{file}"))?
            .lines()
            .map(|line| {
                let mut entry: Value = serde_json::from_str(line)?;
                entry["at"] = "A".into();
                if entry.get("ms").is_some() {
                    entry["ms"] = 0u8.into();
                }
                Ok(entry.to_string())
            })
            .collect()
    }

    /// A fresh temp root named for the test, its `models/` each `(file, fixture)` copied in.
    fn root(name: &str, models: &[(&str, &str)]) -> Result<String, Box<dyn Error>> {
        let dir = env::temp_dir().join(format!("infer-{name}-{}", id()));
        drop(fs::remove_dir_all(&dir));
        fs::create_dir_all(dir.join("models"))?;
        for &(file, fixture) in models {
            let _: u64 = fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests")
                    .join(fixture),
                dir.join("models").join(file),
            )?;
        }
        Ok(dir.to_str().ok_or("temp path")?.to_owned())
    }

    /// `Err` showing both sides unless `got == want`.
    fn same(got: &str, want: &str) -> Outcome {
        if got == want {
            Ok(())
        } else {
            Err(format!("got:\n{got}\nwant:\n{want}").into())
        }
    }

    /// `sandman-infer` over `root` with `envs`, stdin a pipe the guard holds, and the health line it
    /// prints once listening.
    fn start(root: &str, envs: &[(&str, &str)]) -> Result<(Daemon, Value), Box<dyn Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sandman-infer"))
            // not env_clear: llvm-cov reaches the child through LLVM_PROFILE_FILE
            .env("SANDMAN_ROOT", root)
            .env_remove("SANDMAN_LOG_CAP")
            .env_remove("SANDMAN_MODELS")
            .envs(envs.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut daemon = Daemon(child, stdin);
        let mut ready = String::new();
        let _: usize =
            BufReader::new(daemon.0.stdout.take().ok_or("stdout")?).read_line(&mut ready)?;
        Ok((daemon, serde_json::from_str(&ready)?))
    }

    /// `check`, polled every 10 ms for up to `seconds`.
    fn until<F: Fn() -> bool>(seconds: u16, check: F) -> Outcome {
        let polled = (0..seconds.saturating_mul(100)).any(|_| {
            check() || {
                thread::sleep(Duration::from_millis(10));
                false
            }
        });
        Ok(polled.then_some(()).ok_or("timed out")?)
    }

    /// The daemon's health once its load is done.
    fn settled(root: &str, seconds: u16) -> Result<Value, Box<dyn Error>> {
        let health = || call(root, &json!({"op": "health"}));
        until(seconds, || {
            health().is_ok_and(|reply| reply["state"] != "booting")
        })?;
        health()
    }

    /// SIGTERM, so the daemon's coverage counts, then its exit code.
    fn stopped(mut daemon: Daemon) -> Result<String, Box<dyn Error>> {
        Ok(format!(
            "{} {:?}",
            kill("-TERM", &daemon)?,
            daemon.0.wait()?.code()
        ))
    }

    /// The daemon's life: the ready line `booting` with `run/` 0700 and a socket, then `ready`; a
    /// second instance refused; typed errors; a silent or vanished client survived; SIGTERM drains
    /// to exit 0 with its line logged and the socket gone, the log rotated at its cap.
    #[test]
    fn life() -> Outcome {
        let root = root(
            "life",
            &[(DECIDER, "decider.gguf"), (EMBEDDER, "embed.gguf")],
        )?;
        let (daemon, ready) = start(&root, &[("SANDMAN_LOG_CAP", "1")])?;
        let pid = daemon.0.id();
        same(
            &format!(
                "{} {} {} {} {}",
                ready["models"],
                ready["pid"],
                ready["state"],
                ready["version"],
                ready["since"].as_str().map_or(0, str::len)
            ),
            &format!(r#"{{"decide":false,"embed":false}} {pid} "booting" "{VERSION}" 20"#),
        )?;
        let health = settled(&root, 60)?;
        same(
            &format!("{} {}", health["models"], health["state"]),
            r#"{"decide":true,"embed":true} "ready""#,
        )?;
        let again = Command::new(env!("CARGO_BIN_EXE_sandman-infer"))
            .env("SANDMAN_ROOT", &root)
            .output()?;
        same(
            &format!(
                "{:?} {}",
                again.status.code(),
                String::from_utf8(again.stderr)?
            ),
            "Some(1) sandman-infer: already running\n",
        )?;
        let run = format!("{root}/run");
        same(
            &format!(
                "{:o} {}",
                fs::metadata(&run)?.mode() & 0o777,
                fs::metadata(format!("{run}/infer.sock"))?
                    .file_type()
                    .is_socket()
            ),
            "700 true",
        )?;
        same(
            &format!(
                "{}{}{}",
                ask(&root, "not json\n")?,
                ask(&root, "{\"op\":\"nap\"}\n")?,
                ask(&root, "")?
            ),
            "{\"code\":\"bad_request\",\"ok\":false}\n{\"code\":\"unknown_op\",\"ok\":false}\n",
        )?;
        // gone before the reply: the write meets EPIPE, which must not kill the daemon
        UnixStream::connect(format!("{run}/infer.sock"))?.write_all(b"{\"op\":\"health\"}\n")?;
        same(
            &call(&root, &json!({"op": "health"}))?.to_string(),
            &health.to_string(),
        )?;
        same(&stopped(daemon)?, "true Some(0)")?;
        same(
            &format!(
                "{:?} {:?}",
                log(&root, "infer.log.1")?,
                log(&root, "infer.log")?
            ),
            r#"["{\"at\":\"A\",\"event\":\"load\",\"ms\":0,\"op\":\"embed\",\"outcome\":\"loaded\"}"] ["{\"at\":\"A\",\"event\":\"exit\",\"reason\":\"signal TERM\"}"]"#,
        )?;
        same(
            &format!("{:?}", fs::exists(format!("{run}/infer.sock"))),
            "Ok(false)",
        )
    }

    /// Each op over the tiny models, and each of its typed errors: decide renders the official
    /// prompt (escapes and the default instructions seen in the token count), answers per question
    /// in key order, the same request the same nouls; a prompt over 8,192 tokens is refused whole;
    /// embed cuts a text at 1,023 tokens, batches 16 a decode, and L2-normalises.
    #[test]
    fn ops() -> Outcome {
        let root = root(
            "ops",
            &[(DECIDER, "decider.gguf"), (EMBEDDER, "embed.gguf")],
        )?;
        let (daemon, _) = start(&root, &[])?;
        let pid = daemon.0.id();
        drop(settled(&root, 60)?);
        let decide = |state: &str, questions: &Value| {
            call(
                &root,
                &json!({"op": "decide", "questions": questions, "state": state}),
            )
        };
        let tokens = |state: &str, question: &Value| -> Result<u64, Box<dyn Error>> {
            decide(state, &json!({"0": question}))?["usage"]["input_tokens"]
                .as_u64()
                .ok_or_else(|| "no usage".into())
        };
        let noul = json!({"type": "noul"});
        let base = tokens("x", &noul)?;
        same(
            &format!(
                "{} {} {} {} {}",
                // the marker escaped to full-width brackets, 18 tokens of a byte each
                tokens("<|fim_suffix|>", &noul)?.saturating_sub(base),
                // one special token, as the official encoder leaves it
                tokens("<|im_start|>", &noul)?.saturating_sub(base),
                tokens("x", &json!({"instructions": "", "type": "noul"}))?.saturating_sub(base),
                tokens(
                    "x",
                    &json!({"instructions": "Choose the best matching option.", "type": "noul"})
                )?
                .saturating_sub(base),
                tokens("x", &json!({"instructions": "xy", "type": "noul"}))?
            ),
            &format!("17 0 0 0 {}", base.saturating_sub(30)),
        )?;
        let two = json!({"b": {"instructions": "Is it?", "type": "noul"}, "a": noul});
        let first = decide("the state", &two)?;
        same(&decide("the state", &two)?.to_string(), &first.to_string())?;
        let answers = first["answers"].as_object().ok_or("answers")?;
        same(
            &format!(
                "{:?} {} {}",
                answers.keys().collect::<Vec<_>>(),
                answers.values().all(|answer| answer["type"] == "noul"
                    && answer["noul"]
                        .as_f64()
                        .is_some_and(|value| value > 0.0f64 && value < 1.0f64)),
                first["ok"]
            ),
            r#"["a", "b"] true true"#,
        )?;
        let shape = r#"decide takes {"questions":{"<id>":{"instructions"?:str,"type":"noul"}},"state":str}"#;
        let bad = [
            json!({"op": "decide", "questions": {"0": noul}}),
            json!({"op": "decide", "questions": {"0": noul}, "state": 5u8}),
            json!({"op": "decide", "questions": {}, "state": "x"}),
            json!({"op": "decide", "questions": {"": noul}, "state": "x"}),
            json!({"op": "decide", "questions": {"0": {"type": "choice"}}, "state": "x"}),
            json!({"op": "decide", "questions": {"0": {"instructions": 5u8, "type": "noul"}}, "state": "x"}),
            json!({"op": "decide", "questions": {"0": {"criteria": {}, "type": "noul"}}, "state": "x"}),
            json!({"op": "decide", "questions": [noul], "state": "x"}),
        ];
        for request in bad {
            same(
                &call(&root, &request)?.to_string(),
                &json!({"code": "bad_request", "ok": false, "reason": shape}).to_string(),
            )?;
        }
        same(
            &decide(&"x".repeat(9000), &json!({"0": noul}))?.to_string(),
            &json!({"code": "too_long", "ok": false, "reason": format!("{} tokens, over 8192", base.saturating_add(8999))}).to_string(),
        )?;
        let embed = |texts: &Value| call(&root, &json!({"op": "embed", "texts": texts}));
        let long = "y".repeat(1023);
        // 17 texts, two decodes: the first again last, a text cut at 1,023 tokens beside that cut
        let texts: Vec<String> = [
            "alpha",
            "alpha",
            "beta",
            &format!("{long}{}", "z".repeat(1000)),
            &long,
        ]
        .iter()
        .map(|text| (*text).to_owned())
        .chain((0u8..11).map(|index| format!("text {index}")))
        .chain(iter::once("alpha".to_owned()))
        .collect();
        let reply = embed(&json!(texts))?;
        let vectors: Vec<Vec<f64>> = reply["embeddings"]
            .as_array()
            .ok_or("embeddings")?
            .iter()
            .map(|vector| {
                vector
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64)
                    .collect()
            })
            .collect();
        let apart = |left: usize, right: usize| {
            vectors[left]
                .iter()
                .zip(&vectors[right])
                .map(|(one, other)| (one - other).abs())
                .fold(0.0f64, f64::max)
        };
        same(
            &format!(
                "{} {} {} {} {} {} {} {}",
                reply["ok"],
                vectors.len(),
                vectors.iter().all(|vector| vector.len() == 16),
                vectors.iter().all(|vector| (vector
                    .iter()
                    .map(|value| value * value)
                    .sum::<f64>()
                    - 1.0f64)
                    .abs()
                    < 1e-5f64),
                apart(0, 1) < 1e-5f64,
                apart(0, 16) < 1e-5f64,
                apart(3, 4) < 1e-5f64,
                apart(0, 2) > 1e-3f64
            ),
            "true 17 true true true true true true",
        )?;
        same(
            &format!(
                "{}{}{}",
                embed(&json!([]))?,
                embed(&json!([1u8]))?,
                call(&root, &json!({"op": "embed"}))?
            ),
            r#"{"embeddings":[],"ok":true}{"code":"bad_request","ok":false,"reason":"embed takes {\"texts\":[str]}"}{"code":"bad_request","ok":false,"reason":"embed takes {\"texts\":[str]}"}"#,
        )?;
        same(&stopped(daemon)?, "true Some(0)")?;
        let lines = log(&root, "infer.log")?;
        same(
            &lines.get(..3).ok_or("three lines")?.join("\n"),
            &format!(
                "{{\"at\":\"A\",\"event\":\"start\",\"pid\":{pid},\"version\":\"{VERSION}\"}}\n{}\n{}",
                r#"{"at":"A","event":"load","ms":0,"op":"decide","outcome":"loaded"}"#,
                r#"{"at":"A","event":"load","ms":0,"op":"embed","outcome":"loaded"}"#,
            ),
        )
    }

    /// A model missing or unloadable disables its op alone, `degraded` naming which, each load line
    /// `failed` with its reason and each op's reply `no_model` with the same: a GGUF without the
    /// decider's metadata, a file that is no GGUF, one cut inside its metadata, one cut in its
    /// tensors, no file, a decider whose first code's token id is past its vocabulary.
    #[test]
    fn degraded() -> Outcome {
        let fixture = |name: &str| {
            fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests")
                    .join(name),
            )
        };
        let decider = fixture("decider.gguf")?;
        // `pplx_decider.token_ids`' first i32, past its key, type, element type and count
        let key = b"pplx_decider.token_ids";
        let at = decider
            .windows(key.len())
            .position(|window| window == key)
            .ok_or("no token_ids")?
            .saturating_add(key.len() + 16);
        let mut past = decider.clone();
        drop(past.splice(at..at + 4, 1000i32.to_le_bytes()));
        let embedder = fixture("embed.gguf")?;
        let cases = [
            ("bare", &*embedder, Some(&b"not a model"[..])),
            ("cut", decider.get(..64).ok_or("short")?, Some(&*embedder)),
            ("ids", &*past, Some(&*embedder)),
            (
                "tensors",
                decider
                    .get(..decider.len().saturating_sub(4096))
                    .ok_or("short")?,
                None,
            ),
        ];
        let mut seen = Vec::new();
        for (name, decide, embed) in cases {
            let dir = root(&format!("degraded-{name}"), &[])?;
            fs::write(format!("{dir}/models/{DECIDER}"), decide)?;
            if let Some(bytes) = embed {
                fs::write(format!("{dir}/models/{EMBEDDER}"), bytes)?;
            }
            let (daemon, _) = start(&dir, &[])?;
            let health = settled(&dir, 60)?;
            let models = format!("{dir}/models");
            let reply = |op: &str| -> Result<String, Box<dyn Error>> {
                let answer = call(
                    &dir,
                    &json!({"op": op, "questions": {"0": {"type": "noul"}}, "state": "x", "texts": ["x"]}),
                )?;
                Ok(if answer["ok"] == true {
                    "ok".to_owned()
                } else {
                    format!("{} {}", answer["code"], answer["reason"]).replace(&models, "<models>")
                })
            };
            seen.push(format!(
                "{name}: {} {} {} {}",
                health["state"],
                health["models"],
                reply("decide")?,
                reply("embed")?
            ));
            same(&stopped(daemon)?, "true Some(0)")?;
            let lines = log(&dir, "infer.log")?;
            seen.push(
                lines
                    .get(1..3)
                    .ok_or("load lines")?
                    .join(" ")
                    .replace(&models, "<models>"),
            );
        }
        same(
            &seen.join("\n"),
            &[
                r#"bare: "degraded" {"decide":false,"embed":false} "no_model" "not a joint-question-inline-readout-v1 model: pplx_decider metadata missing or invalid" "no_model" "<models>/qwen3-embedding-0.6b-q8_0.gguf: not a GGUF file""#,
                r#"{"at":"A","event":"load","ms":0,"op":"decide","outcome":"failed","reason":"not a joint-question-inline-readout-v1 model: pplx_decider metadata missing or invalid"} {"at":"A","event":"load","ms":0,"op":"embed","outcome":"failed","reason":"<models>/qwen3-embedding-0.6b-q8_0.gguf: not a GGUF file"}"#,
                r#"cut: "degraded" {"decide":false,"embed":true} "no_model" "<models>/decider-flash-v1.2-q8_0.gguf: failed to fill whole buffer" ok"#,
                r#"{"at":"A","event":"load","ms":0,"op":"decide","outcome":"failed","reason":"<models>/decider-flash-v1.2-q8_0.gguf: failed to fill whole buffer"} {"at":"A","event":"load","ms":0,"op":"embed","outcome":"loaded"}"#,
                r#"ids: "degraded" {"decide":false,"embed":true} "no_model" "not a joint-question-inline-readout-v1 model: pplx_decider metadata missing or invalid" ok"#,
                r#"{"at":"A","event":"load","ms":0,"op":"decide","outcome":"failed","reason":"not a joint-question-inline-readout-v1 model: pplx_decider metadata missing or invalid"} {"at":"A","event":"load","ms":0,"op":"embed","outcome":"loaded"}"#,
                r#"tensors: "degraded" {"decide":false,"embed":false} "no_model" "null result from llama cpp" "no_model" "<models>/qwen3-embedding-0.6b-q8_0.gguf: No such file or directory (os error 2)""#,
                r#"{"at":"A","event":"load","ms":0,"op":"decide","outcome":"failed","reason":"null result from llama cpp"} {"at":"A","event":"load","ms":0,"op":"embed","outcome":"failed","reason":"<models>/qwen3-embedding-0.6b-q8_0.gguf: No such file or directory (os error 2)"}"#,
            ]
            .join("\n"),
        )
    }

    /// Booting: a model file that blocks its open (a FIFO) holds the load, so health says
    /// `booting` and each op is refused `booting` until it returns.
    #[test]
    fn booting() -> Outcome {
        let dir = root("booting", &[(EMBEDDER, "embed.gguf")])?;
        let fifo = format!("{dir}/models/{DECIDER}");
        same(
            &format!("{:?}", Command::new("mkfifo").arg(&fifo).status()?.code()),
            "Some(0)",
        )?;
        let (daemon, ready) = start(&dir, &[])?;
        let booting = call(&dir, &json!({"op": "health"}))?;
        same(
            &format!(
                "{} {} {} {}",
                ready["state"],
                booting["state"],
                call(&dir, &json!({"op": "decide"}))?,
                call(&dir, &json!({"op": "embed"}))?
            ),
            r#""booting" "booting" {"code":"booting","ok":false} {"code":"booting","ok":false}"#,
        )?;
        OpenOptions::new()
            .write(true)
            .open(&fifo)?
            .write_all(b"nope")?;
        let health = settled(&dir, 60)?;
        same(
            &format!("{} {}", health["state"], health["models"]),
            r#""degraded" {"decide":false,"embed":true}"#,
        )?;
        same(&stopped(daemon)?, "true Some(0)")
    }

    /// The drain: SIGTERM while the worker is writing a reply its client reads slowly, 4 KB every
    /// 50 ms (each read well inside the 1 s write timeout, the whole well past the stop); health
    /// says `stopping`, a new op is refused `stopping`, the reply in hand is finished whole, an op
    /// queued behind it is answered `stopping`, then exit 0.
    #[test]
    fn drain() -> Outcome {
        let dir = root(
            "drain",
            &[(DECIDER, "decider.gguf"), (EMBEDDER, "embed.gguf")],
        )?;
        let (mut daemon, _) = start(&dir, &[])?;
        drop(settled(&dir, 60)?);
        let sock = format!("{dir}/run/infer.sock");
        let texts: Vec<String> = (0u16..3000).map(|index| format!("text {index}")).collect();
        // far past a socket's buffer: the worker blocks writing it until this end reads
        let mut slow = UnixStream::connect(&sock)?;
        writeln!(slow, "{}", json!({"op": "embed", "texts": texts}))?;
        let mut head = [0u8; 1];
        slow.read_exact(&mut head)?;
        let reader = thread::spawn(move || -> io::Result<Vec<u8>> {
            let (mut rest, mut chunk) = (vec![head[0]], [0u8; 4096]);
            loop {
                thread::sleep(Duration::from_millis(50));
                match slow.read(&mut chunk)? {
                    0 => return Ok(rest),
                    size => rest.extend_from_slice(chunk.get(..size).unwrap_or_default()),
                }
            }
        });
        let mut queued = UnixStream::connect(&sock)?;
        writeln!(
            queued,
            "{}",
            json!({"op": "decide", "questions": {"0": {"type": "noul"}}, "state": "x"})
        )?;
        // the accept loop is one thread: this answered, the decide before it is queued
        drop(call(&dir, &json!({"op": "health"}))?);
        same(&kill("-TERM", &daemon)?.to_string(), "true")?;
        until(10, || {
            call(&dir, &json!({"op": "health"})).is_ok_and(|reply| reply["state"] == "stopping")
        })?;
        let refused = call(&dir, &json!({"op": "embed", "texts": []}))?;
        let whole: Value =
            serde_json::from_slice(&reader.join().map_err(|_panic| "reader panicked")??)?;
        let mut answer = String::new();
        let _: usize = queued.read_to_string(&mut answer)?;
        same(
            &format!(
                "{refused} {} {answer}{:?}",
                whole["embeddings"].as_array().map_or(0, Vec::len),
                daemon.0.wait()?.code()
            ),
            "{\"code\":\"stopping\",\"ok\":false} 3000 {\"code\":\"stopping\",\"ok\":false}\nSome(0)",
        )?;
        same(
            log(&dir, "infer.log")?.last().map_or("", String::as_str),
            r#"{"at":"A","event":"exit","reason":"signal TERM"}"#,
        )
    }

    /// EOF on stdin, `serve`'s pipe closed by its death, stops the daemon as a signal does: exit 0,
    /// its exit line `stdin closed`.
    #[test]
    fn orphan() -> Outcome {
        let dir = root("orphan", &[(EMBEDDER, "embed.gguf")])?;
        let (mut daemon, _) = start(&dir, &[])?;
        drop(daemon.1.take());
        same(&format!("{:?}", daemon.0.wait()?.code()), "Some(0)")?;
        same(
            log(&dir, "infer.log")?.last().map_or("", String::as_str),
            r#"{"at":"A","event":"exit","reason":"stdin closed"}"#,
        )
    }

    /// A worker panic ends the process, exit 1, so `serve` respawns it: never a live socket whose
    /// model ops go unanswered. The trigger is a decider file gone between the metadata read and
    /// llama.cpp's load (a FIFO unlinked once open), which llama-cpp-2's `debug_assert` of the
    /// path turns into a panic in this, the suite's, build.
    #[test]
    fn dies() -> Outcome {
        let dir = root("dies", &[])?;
        let fifo = format!("{dir}/models/{DECIDER}");
        same(
            &format!("{:?}", Command::new("mkfifo").arg(&fifo).status()?.code()),
            "Some(0)",
        )?;
        let (mut daemon, _) = start(&dir, &[])?;
        let mut pipe = OpenOptions::new().write(true).open(&fifo)?;
        fs::remove_file(&fifo)?;
        // the metadata read, then the reader closes its end: the rest meets EPIPE
        drop(pipe.write_all(&fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/decider.gguf"),
        )?));
        until(10, || call(&dir, &json!({"op": "health"})).is_err())?;
        let text = fs::read_to_string(format!("{dir}/run/infer.log"))?;
        same(
            &format!(
                "{:?} {} {}",
                daemon.0.wait()?.code(),
                text.contains("does not exist"),
                log(&dir, "infer.log")?.last().map_or("", String::as_str)
            ),
            r#"Some(1) true {"at":"A","event":"exit","reason":"worker died"}"#,
        )
    }

    /// A client trickling its line holds the accept loop no longer than the line's 1 s deadline:
    /// health behind it answers within 1.5 s. A line over the 1 MiB cap is closed unanswered, one
    /// just under it answered.
    #[test]
    fn trickle() -> Outcome {
        let dir = root("trickle", &[])?;
        let (daemon, _) = start(&dir, &[])?;
        drop(settled(&dir, 60)?);
        let sock = format!("{dir}/run/infer.sock");
        let mut slow = UnixStream::connect(&sock)?;
        let dripping = thread::spawn(move || {
            for _ in 0u8..6 {
                drop(slow.write_all(b" "));
                thread::sleep(Duration::from_millis(500));
            }
        });
        let begun = Instant::now();
        let health = call(&dir, &json!({"op": "health"}))?;
        let spent = begun.elapsed();
        let padded = |size: usize| -> io::Result<String> {
            let mut stream = UnixStream::connect(&sock)?;
            drop(writeln!(
                stream,
                "{}",
                json!({"op": "health", "pad": "x".repeat(size)})
            ));
            let mut reply = String::new();
            drop(stream.read_to_string(&mut reply));
            Ok(reply)
        };
        same(
            &format!(
                "{} {} {:?} {}",
                health["ok"],
                spent < Duration::from_millis(1500),
                padded(1usize << 20u8)?,
                padded((1usize << 20u8) - 64)?.contains(r#""ok":true"#)
            ),
            r#"true true "" true"#,
        )?;
        drop(dripping.join());
        same(&stopped(daemon)?, "true Some(0)")
    }

    /// A SIGKILL leaves the socket behind; the next start, holding the lock, binds over it.
    #[test]
    fn stale() -> Outcome {
        let dir = root("stale", &[(EMBEDDER, "embed.gguf")])?;
        let (mut daemon, _) = start(&dir, &[])?;
        same(&kill("-KILL", &daemon)?.to_string(), "true")?;
        let _: ExitStatus = daemon.0.wait()?;
        let sock = format!("{dir}/run/infer.sock");
        same(
            &fs::metadata(&sock)?.file_type().is_socket().to_string(),
            "true",
        )?;
        let (again, ready) = start(&dir, &[])?;
        same(
            &format!("{} {}", ready["state"], settled(&dir, 60)?["state"]),
            r#""booting" "degraded""#,
        )?;
        same(&stopped(again)?, "true Some(0)")
    }

    /// The start refuses a socket path over 103 bytes and a `run/` others can reach (the root
    /// `$HOME/.sandman` without `SANDMAN_ROOT`); a panic (here the ready line into a closed stdout)
    /// is logged before the process dies.
    #[test]
    fn refuse() -> Outcome {
        let dir = root("refuse", &[])?;
        let run = |command: &mut Command| -> Result<String, Box<dyn Error>> {
            let out = command.env_remove("SANDMAN_LOG_CAP").output()?;
            Ok(format!(
                "{:?} {}",
                out.status.code(),
                String::from_utf8(out.stderr)?
            ))
        };
        let bin = env!("CARGO_BIN_EXE_sandman-infer");
        let deep = format!("{dir}/{}", "x".repeat(80));
        same(
            &run(Command::new(bin).env("SANDMAN_ROOT", &deep))?,
            &format!(
                "Some(1) sandman-infer: {deep}/run/infer.sock is over a unix socket's 103 bytes; shorten SANDMAN_ROOT\n"
            ),
        )?;
        let open = format!("{dir}/.sandman/run");
        fs::create_dir_all(&open)?;
        fs::set_permissions(&open, Permissions::from_mode(0o755))?;
        same(
            &run(Command::new(bin)
                .env_remove("SANDMAN_ROOT")
                .env("HOME", &dir))?,
            &format!("Some(1) sandman-infer: {open} must be this user's, mode 0700\n"),
        )?;
        let (reader, writer) = io::pipe()?;
        drop(reader);
        let status = Command::new(bin)
            .env("SANDMAN_ROOT", &dir)
            .stdout(writer)
            .status()?;
        let text = fs::read_to_string(format!("{dir}/run/infer.log"))?;
        same(
            &format!(
                "{:?} {} {}",
                status.code(),
                text.contains(r#","event":"panic","reason":"panicked at "#),
                text.contains("failed printing to stdout")
            ),
            "Some(101) true true",
        )
    }

    /// Parity with the real models, run only when `SANDMAN_INFER_PARITY` is
    /// `<decider.gguf>:<embedding.gguf>:<dir of syn.jsonl and syn.ollama.jsonl>`: per golden case
    /// (`golden.py`, the official encoder's ids and markers), the daemon's nouls equal those of the
    /// golden ids decoded in process (so its prompt tokenises to the same ids) and its token count
    /// theirs; `syn-a` and `syn-b` within 0.01 of the native run's; each embedding at cosine
    /// 0.9999 or more with Ollama's. It prints what it measured.
    #[test]
    fn parity() -> Outcome {
        let Ok(paths) = env::var("SANDMAN_INFER_PARITY") else {
            return Ok(());
        };
        let [decider, embedder, data] = paths.split(':').collect::<Vec<_>>()[..] else {
            return Err("SANDMAN_INFER_PARITY: <decider.gguf>:<embedding.gguf>:<syn dir>".into());
        };
        let dir = root("parity", &[])?;
        symlink(decider, format!("{dir}/models/{DECIDER}"))?;
        symlink(embedder, format!("{dir}/models/{EMBEDDER}"))?;
        let (daemon, _) = start(&dir, &[])?;
        same(
            settled(&dir, 300)?["state"].as_str().unwrap_or_default(),
            "ready",
        )?;
        let golden: Value = serde_json::from_reader(File::open(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden.json"),
        )?)?;
        let ms = |begun: Instant| begun.elapsed().as_secs_f64() * 1000.0f64;
        let median = |mut runs: Vec<f64>| {
            runs.sort_by(f64::total_cmp);
            runs.len()
                .checked_div(2)
                .and_then(|middle| runs.get(middle))
                .copied()
                .unwrap_or_default()
        };
        // the reference: each case's golden ids in one batch, the logits at its golden markers
        send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
        let backend = LlamaBackend::init()?;
        let model = LlamaModel::load_from_file(
            &backend,
            decider,
            &LlamaModelParams::default().with_n_gpu_layers(999),
        )?;
        let mut context = model.new_context(
            &backend,
            LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(8192))
                .with_n_batch(8192)
                .with_n_ubatch(8192)
                .with_n_seq_max(1),
        )?;
        let temperature = golden["temperature"].as_f64().ok_or("temperature")?;
        let [no, yes] = [0, 1].map(|index| {
            golden["token_ids"][index]
                .as_u64()
                .and_then(|token| usize::try_from(token).ok())
                .unwrap_or_default()
        });
        let (mut versus_reference, mut versus_native, mut counts) = (0.0f64, 0.0f64, true);
        let mut decide_ms = Vec::new();
        for case in golden["cases"].as_array().ok_or("cases")? {
            let ids: Vec<LlamaToken> = case["ids"]
                .as_array()
                .ok_or("ids")?
                .iter()
                .filter_map(|token| i32::try_from(token.as_i64()?).ok())
                .map(LlamaToken)
                .collect();
            let markers: Vec<i32> = case["markers"]
                .as_array()
                .ok_or("markers")?
                .iter()
                .filter_map(|marker| i32::try_from(marker.as_i64()?).ok())
                .collect();
            context.clear_kv_cache();
            let mut batch = LlamaBatch::new(ids.len(), 1);
            for (position, &token) in (0i32..).zip(&ids) {
                batch.add(token, position, &[0i32], markers.contains(&position))?;
            }
            context.decode(&mut batch)?;
            let reference: Vec<f64> = markers
                .iter()
                .map(|&marker| {
                    let logits = context.get_logits_ith(marker);
                    1.0f64
                        / (1.0f64
                            + ((f64::from(logits[no]) - f64::from(logits[yes])) / temperature)
                                .exp())
                })
                .collect();
            let mut request = case["request"].clone();
            request["op"] = "decide".into();
            let reply = call(&dir, &request)?;
            let nouls: Vec<f64> = reply["answers"]
                .as_object()
                .ok_or("answers")?
                .values()
                .filter_map(|answer| answer["noul"].as_f64())
                .collect();
            counts &= reply["usage"]["input_tokens"].as_u64() == u64::try_from(ids.len()).ok()
                && nouls.len() == reference.len();
            for (got, want) in nouls.iter().zip(&reference) {
                versus_reference = versus_reference.max((got - want).abs());
            }
            for (got, want) in nouls
                .iter()
                .zip(case["nouls"].as_array().into_iter().flatten())
            {
                versus_native = versus_native.max((got - want.as_f64().unwrap_or(f64::NAN)).abs());
            }
            if case["name"] == "syn-a" {
                for _ in 0u8..10 {
                    let begun = Instant::now();
                    drop(call(&dir, &request)?);
                    decide_ms.push(ms(begun));
                }
            }
        }
        drop(context);
        drop(model);
        let lines = |file: &str| -> Result<Vec<Value>, Box<dyn Error>> {
            fs::read_to_string(format!("{data}/{file}"))?
                .lines()
                .map(|line| Ok(serde_json::from_str(line)?))
                .collect()
        };
        let (texts, ollama) = (lines("syn.jsonl")?, lines("syn.ollama.jsonl")?);
        let reply = call(&dir, &json!({"op": "embed", "texts": texts}))?;
        let cosine = |got: &Value, want: &Value| {
            let [one, other]: [Vec<f64>; 2] = [got, want].map(|vector| {
                vector
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64)
                    .collect()
            });
            let norm =
                |vector: &[f64]| vector.iter().map(|value| value * value).sum::<f64>().sqrt();
            one.iter()
                .zip(&other)
                .map(|(left, right)| left * right)
                .sum::<f64>()
                / (norm(&one) * norm(&other))
        };
        let embeddings = reply["embeddings"].as_array().ok_or("embeddings")?;
        let lowest = embeddings
            .iter()
            .zip(&ollama)
            .map(|(got, want)| cosine(got, want))
            .fold(f64::INFINITY, f64::min);
        let sixteen = json!({"op": "embed", "texts": texts.get(..16).ok_or("16 texts")?});
        let embed_ms: Vec<f64> = iter::repeat_with(|| {
            let begun = Instant::now();
            call(&dir, &sixteen).map(|_| ms(begun))
        })
        .take(5)
        .collect::<Result<_, _>>()?;
        let loaded = fs::read_to_string(format!("{dir}/run/infer.log"))?
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|line| line["event"] == "load")
            .map(|line| format!("{} {} ms", line["op"], line["ms"]))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "parity: max |delta noul| {versus_reference:.2e} vs the golden ids, {versus_native:.2e} vs the native run; min cosine {lowest:.8} over {} texts; load {loaded}; decide syn-a p50 {:.1} ms; embed 16 p50 {:.1} ms",
            embeddings.len(),
            median(decide_ms),
            median(embed_ms)
        );
        same(&stopped(daemon)?, "true Some(0)")?;
        same(
            &format!(
                "{counts} {} {} {} {}",
                versus_reference < 1e-4,
                versus_native <= 0.01,
                embeddings.len() == ollama.len(),
                lowest >= 0.9999
            ),
            "true true true true true",
        )
    }
}
