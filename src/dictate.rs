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
    /// arrives, `on_stop` runs once when the recognizer ends.
    pub fn start<F, G>(on_text: F, on_stop: G) -> Result<Self>
    where
        F: FnMut(String) + Send + 'static,
        G: FnOnce() + Send + 'static,
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
        // must not keep recording: the operating system takes it down with us.
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| {
                format!(
                    "could not start `{program} --mic` — install whistle, or point \
                     EXP1_DICTATE at it"
                )
            })?;
        let stdout = child.stdout.take().context("the recognizer has no output")?;
        std::thread::spawn(move || {
            let mut on_text = on_text;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Some(text) = text_of(&line) {
                    on_text(text);
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
fn text_of(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let text = value.get("text")?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recognizer reports both the committed text and the words still in
    /// flight; only the committed part belongs in the prompt.
    #[test]
    fn only_committed_text_is_taken() {
        assert_eq!(
            text_of(r#"{"text":"hello there","pending":"and","language":"en"}"#).as_deref(),
            Some("hello there")
        );
        assert_eq!(text_of(r#"{"text":"  ","pending":"and"}"#), None);
        assert_eq!(text_of(r#"{"pending":"and"}"#), None);
        // Anything that is not one of our lines is not a transcript.
        assert_eq!(text_of("whistle: recording with pw-record"), None);
        assert_eq!(text_of(""), None);
    }
}
