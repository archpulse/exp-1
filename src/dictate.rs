//! Live dictation: the microphone goes to `whistle --mic`, and every line it
//! prints is one piece of committed text, so the words appear while they are
//! still being spoken.
//!
//! The command is `EXP1_DICTATE` (default `whistle`), the language
//! `EXP1_DICTATE_LANG` (default: whatever it detects), bias words
//! `EXP1_DICTATE_KEYWORDS`, and the audio source `WHISTLE_SOURCE` (default: the
//! system's own input).

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

/// A running recognizer. Dropping it without `stop` leaves the process to the
/// operating system, which is what happens when the app is killed.
pub struct Dictation {
    child: Child,
}

impl Dictation {
    /// Starts the recognizer. `on_text` gets every piece of committed text as it
    /// arrives, `on_partial` the words still in flight (the recognizer prints
    /// both every second, so the interface can show speech while it happens),
    /// and `on_stop` runs once when the recognizer ends.
    pub fn start<F, G, H, P>(on_text: F, on_failure: G, on_stop: H, on_partial: P) -> Result<Self>
    where
        F: FnMut(String) + Send + 'static,
        G: FnMut(String) + Send + 'static,
        H: FnOnce() + Send + 'static,
        P: FnMut(String) + Send + 'static,
    {
        let program = program();
        let mut command = Command::new(&program);
        command.arg("--mic");
        if let Ok(language) = std::env::var("EXP1_DICTATE_LANG")
            && !language.trim().is_empty()
        {
            command.arg("-l").arg(language.trim());
        }
        if let Ok(keywords) = std::env::var("EXP1_DICTATE_KEYWORDS")
            && !keywords.trim().is_empty()
        {
            command.arg("-k").arg(keywords.trim());
        }
        // If the interface dies without stopping the dictation, the recognizer
        // must not keep recording: on Linux the kernel takes it down with us
        // (`prctl` has no macOS equivalent).
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        // The recognizer's own complaint is what tells a microphone that hears
        // nothing from a build that has no `--mic` at all.
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "could not start `{program} --mic` — point EXP1_DICTATE at a \
                     recognizer that streams JSON lines"
                )
            })?;
        let stdout = child.stdout.take().context("the recognizer has no output")?;
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            let mut on_text = on_text;
            let mut on_failure = on_failure;
            let mut on_partial = on_partial;
            let mut heard = false;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Some((text, pending)) = parts_of(&line) {
                    if !text.is_empty() {
                        heard = true;
                        on_text(text);
                    }
                    on_partial(pending);
                }
            }
            // A recognizer that ends without a word may simply have heard
            // nothing; if it also complained, that complaint is the answer.
            if !heard && let Some(mut errors) = stderr {
                let mut complaint = String::new();
                use std::io::Read;
                let _ = errors.read_to_string(&mut complaint);
                let complaint = complaint.trim();
                if !complaint.is_empty() {
                    on_failure(complaint.to_string());
                }
            }
            on_stop();
        });
        Ok(Self { child })
    }

    /// Stops the dictation: a newline on stdin ends the stream, which lets the
    /// recognizer flush the words still in flight before it exits.
    pub fn stop(mut self) {
        if let Some(stdin) = self.child.stdin.as_mut() {
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
    }
}

/// Where the recognizer is: what the environment says, then the user-local
/// spot a hand-built binary usually ends up in, then whatever PATH finds.
fn program() -> String {
    if let Ok(custom) = std::env::var("EXP1_DICTATE")
        && !custom.trim().is_empty()
    {
        return custom.trim().to_string();
    }
    if let Some(home) = std::env::var_os("HOME") {
        let local = std::path::PathBuf::from(home).join(".local/bin/whistle");
        if local.is_file() {
            return local.display().to_string();
        }
    }
    "whistle".to_string()
}

impl Drop for Dictation {
    /// A recognizer nobody stopped is killed, so quitting mid-dictation does
    /// not leave the microphone on.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// The committed text of one update line, trimmed; `None` when there is none.
/// The two halves of one update: what is committed, and what is still in
/// flight. Both are printed every second while the microphone is open.
fn parts_of(line: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    (value.get("text").is_some() || value.get("pending").is_some())
        .then(|| (field("text"), field("pending")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recognizer reports the committed text and the words still in
    /// flight; the prompt takes the first, the preview the second.
    #[test]
    fn both_halves_of_an_update_are_read() {
        let line = r#"{"text":"hello there","pending":"and","language":"en"}"#;
        assert_eq!(parts_of(line), Some(("hello there".into(), "and".into())));
        // Blank means nothing: an empty preview, or nothing to insert.
        assert_eq!(parts_of(r#"{"text":"  ","pending":"and"}"#), Some((String::new(), "and".into())));
        assert_eq!(parts_of(r#"{"pending":"and"}"#), Some((String::new(), "and".into())));
        // Anything that is not one of our lines is not a transcript.
        assert_eq!(parts_of(r#"{"language":"en"}"#), None);
        assert_eq!(parts_of("whistle: recording with pw-record"), None);
        assert_eq!(parts_of(""), None);
    }
}
