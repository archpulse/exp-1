//! wrapping and clipping text for the terminal.

//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.
use crate::md;


/// Splits text into lines no wider than `width`, breaking at spaces where it
/// can, so a long tool output or prompt is wrapped instead of cut off at the
/// edge of the terminal.
pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    for word in text.split_inclusive(' ') {
        let mut chars: Vec<char> = word.chars().collect();
        // A word longer than the line is cut rather than allowed to overflow.
        while chars.len() > width {
            let rest: Vec<char> = chars.drain(width..).collect();
            if used > 0 {
                lines.push(std::mem::take(&mut current));
                used = 0;
            }
            lines.push(chars.iter().collect());
            chars = rest;
        }
        let word_width = chars.len();
        if used + word_width > width && used > 0 {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        current.extend(chars);
        used += word_width;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// The same for styled runs: a line of markdown is split at the width so the
/// scroll offset keeps counting real screen lines.
/// One piece of a run: the style and the text, plus whatever else the run
/// carried, which for a link is the address a click needs.
fn wrapped_run(run: &md::Run, text: &str) -> md::Run {
    let mut piece = md::Run::new(run.style, text);
    piece.url = run.url.clone();
    piece
}

pub(crate) fn wrap_runs(runs: &[md::Run], width: usize) -> Vec<Vec<md::Run>> {
    if width == 0 {
        return vec![runs.to_vec()];
    }
    let mut out: Vec<Vec<md::Run>> = Vec::new();
    let mut current: Vec<md::Run> = Vec::new();
    let mut used = 0usize;
    for run in runs {
        let mut chunk = String::new();
        for ch in run.text.chars() {
            if used == width {
                if !chunk.is_empty() {
                    current.push(wrapped_run(run, &chunk));
                    chunk.clear();
                }
                out.push(std::mem::take(&mut current));
                used = 0;
            }
            chunk.push(ch);
            used += 1;
        }
        if !chunk.is_empty() {
            current.push(wrapped_run(run, &chunk));
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// The word being typed when it is an `@` mention, without the `@`.
pub(crate) fn at_token(text: &str) -> Option<String> {
    let word = text.split_whitespace().last()?;
    word.strip_prefix('@').map(str::to_string)
}
