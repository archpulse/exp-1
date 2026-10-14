//! running commands, in the foreground or in the background.

//! Tool implementations: read, write, edit, shell, ast.
//!
//! All paths are resolved against the session working directory. Nothing is
//! sandboxed: commands run as the user.

use super::*;
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;
use tokio::process::Command;

/// A foreground command still running after this long is promoted to a
/// background task, so the agent is never blocked on something slow. A
/// background task has no clock on it: builds and test suites run for minutes,
/// and the old 120s axe was cutting them off mid-flight.

// ---------------------------------------------------------------- shell

fn argv_from(args: &Value) -> Result<Vec<String>, String> {
    if let Some(array) = args.get("command").and_then(Value::as_array) {
        let argv: Vec<String> = array
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if !argv.is_empty() {
            return Ok(argv);
        }
    }
    if let Some(cmd) = args.get("cmd").and_then(Value::as_str)
        && !cmd.trim().is_empty()
    {
        return Ok(vec!["/bin/sh".into(), "-c".into(), cmd.to_string()]);
    }
    Err("shell: 'command' is required".to_string())
}

/// Re-reads a previous command's output, or polls a background task.
pub async fn read_output(registry: &Shared, args: &Value) -> Outcome {
    let Some(id) = args.get("output_id").and_then(Value::as_str) else {
        return err("shell: 'output_id' is required");
    };
    let limit = args.get("limit").and_then(Value::as_u64).map(|v| v as usize);

    let stored = { registry.lock().await.outputs.get(id).cloned() };
    if let Some(stored) = stored {
        return Outcome {
            output: format!(
                "re-read of '{}' with limit {}:\n{}",
                stored.label,
                limit.unwrap_or(DEFAULT_LINES),
                clamp_output(&stored.text, limit)
            ),

            background_id: None,

        };
    }

    // Otherwise it must be a background task.
    let mut guard = registry.lock().await;
    let Some(task) = guard.background.get_mut(id) else {
        let known: Vec<String> = guard
            .outputs
            .keys()
            .chain(guard.background.keys())
            .cloned()
            .collect();
        return err(format!(
            "shell: unknown output_id '{id}'{}\nstart a command first, then poll with the id the \
             shell tool returned",
            if known.is_empty() {
                "; no stored output yet".to_string()
            } else {
                format!("; known ids: {}", known.join(", "))
            }
        ));
    };
    let done = task.handle.as_ref().is_some_and(|h| h.is_finished());
    if !done {
        return Outcome {
            output: format!(
                "'{}' is still running ({}s elapsed); call shell again with output_id '{id}' to check again",
                task.command,
                task.started.elapsed().as_secs()
            ),

            background_id: None,

        };
    }
    let command = task.command.clone();
    let handle = task.handle.take().expect("finished");
    drop(guard);

    let output = match handle.await {
        Ok(output) => output,
        Err(e) if e.is_cancelled() => "exit_code: killed\ncommand was stopped".to_string(),
        Err(_) => "exit_code: failed\ntask panicked".to_string(),
    };

    let mut guard = registry.lock().await;
    guard.background.remove(id);
    guard.outputs.insert(
        id.to_string(),
        StoredOutput {
            label: command,
            text: output.clone(),
        },
    );
    Outcome {
        output: clamp_output(&output, limit),
        background_id: None,
    }
}

/// Removes ANSI escape sequences. `TERM=dumb` stops most tools from colouring
/// their output, but not all of them, and escape bytes are pure noise in a
/// model's context.
pub(crate) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            // CSI: ESC [ … a byte in 0x40..=0x7e ends it.
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: ESC ] … ends at BEL or ESC \.
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other escape is two characters.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

fn render_output(output: &std::process::Output) -> String {
    let mut body = String::new();
    let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    if !stdout.is_empty() {
        body.push_str(stdout.trim_end());
    }
    if !stderr.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("[stderr]\n");
        body.push_str(stderr.trim_end());
    }
    if body.is_empty() {
        body.push_str("(no output)");
    }
    format!("exit_code: {}\n{body}", output.status.code().unwrap_or(-1))
}

pub async fn shell(cwd: &Path, registry: &Shared, args: &Value) -> Outcome {
    // Only an explicit, non-empty id means "read previous output"; a model that
    // sends an empty one alongside a command means to run that command.
    if args
        .get("output_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty())
    {
        return read_output(registry, args).await;
    }
    let argv = match argv_from(args) {
        Ok(argv) => argv,
        Err(message) => return err(message),
    };
    let limit = args.get("limit").and_then(Value::as_u64).map(|v| v as usize);
    let background = args
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let label = argv.join(" ");

    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env("TERM", "dumb");
    // Dropping the task — the turn being torn down, the server shutting down —
    // stops the process instead of leaving it running with nobody listening.
    command.kill_on_drop(true);

    if background {
        // The agent keeps working; the model polls this id when it needs the result.
        let id = {
            let mut guard = registry.lock().await;
            guard.next_id("bg")
        };
        let handle = tokio::spawn(async move {
            match command.output().await {
                Err(e) => format!("exit_code: spawn_failed\n{e}"),
                Ok(output) => render_output(&output),
            }
        });
        let mut guard = registry.lock().await;
        guard.background.insert(
            id.clone(),
            BackgroundTask {
                command: label.clone(),
                started: Instant::now(),
                handle: Some(handle),
            },
        );
        return Outcome {
            output: format!(
                "started in background as '{id}'\ncall shell again with {{\"output_id\": \"{id}\"}} to poll it"
            ),
            // The watcher needs this to report the finish back to the agent.
            background_id: Some(id),
        };
    }

    // Foreground: the agent waits, but only for so long. A command still
    // running after a minute is handed to the background so nothing blocks.
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return err(format!("shell: could not start '{label}': {e}")),
    };
    let mut waiter = Box::pin(async move { child.wait_with_output().await });

    match tokio::time::timeout(FOREGROUND_LIMIT, &mut waiter).await {
        Ok(Ok(output)) => {
            let text = render_output(&output);
            let id = {
                let mut guard = registry.lock().await;
                let id = guard.next_id("out");
                guard.outputs.insert(
                    id.clone(),
                    StoredOutput {
                        label: label.clone(),
                        text: text.clone(),
                    },
                );
                id
            };
            Outcome {
                output: format!(
                    "{}\noutput_id: {id} — re-read with a different limit if this was cut short",
                    clamp_output(&text, limit)
                ),
                background_id: None,
            }
        }
        Ok(Err(e)) => err(format!("shell: '{label}' failed: {e}")),
        Err(_) => {
            // Promote the still-running process instead of killing it.
            let id = {
                let mut guard = registry.lock().await;
                let id = guard.next_id("bg");
                let handle = tokio::spawn(async move {
                    match waiter.await {
                        Err(e) => format!("exit_code: spawn_failed\n{e}"),
                        Ok(output) => render_output(&output),
                    }
                });
                guard.background.insert(
                    id.clone(),
                    BackgroundTask {
                        command: label.clone(),
                        started: Instant::now(),
                        handle: Some(handle),
                    },
                );
                id
            };
            Outcome {
                output: format!(
                    "'{label}' is still running after {}s, so it was moved to the background as \
                     '{id}'. Carry on; you will be told when it finishes, or poll it with \
                     {{\"output_id\": \"{id}\"}}.",
                    FOREGROUND_LIMIT.as_secs()
                ),
                background_id: Some(id),
            }
        }
    }
}

