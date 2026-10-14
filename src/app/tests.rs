//! Terminal UI state and rendering for exp-1 agent.
//!
//! Layout follows the OpenAI Codex TUI: a `>_` masthead, submitted prompts as
//! highlighted bands in the transcript, a completion popup or selection picker
//! above the input, a padded input band with a blinking caret, then a model line
//! and a hint line. Two threads exist — the main conversation and a side thread
//! opened with ctrl+/.
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::md;
use crate::protocol::{InputItem, Usage};

use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// The level a switch lands on: the one in use when the model takes it, the
    /// model's default when that fits, and the first level otherwise.
    #[test]
    fn settling_an_effort_follows_the_model() {
        let levels = |list: &[&str]| list.iter().map(|level| level.to_string()).collect::<Vec<_>>();
        let grok = levels(&["low", "medium", "high", "xhigh"]);
        assert_eq!(settle_effort(&grok, Some("medium"), "high"), "high");
        assert_eq!(settle_effort(&grok, Some("medium"), "max"), "medium");
        assert_eq!(settle_effort(&grok, None, "max"), "low");
        assert_eq!(settle_effort(&grok, Some("nonsense"), "max"), "low");
        // A model with no levels keeps whatever it is given.
        assert_eq!(settle_effort(&[], None, "max"), "max");
    }

    /// A link in a message is recorded where it was drawn, so a click can open
    /// it: the label is what a reader sees, the address is what it points at.
    #[test]
    fn a_link_records_where_it_was_drawn() {
        let theme = &THEMES[0];
        let text = "see [docs](https://example.com/x) for more";
        let msg = Msg {
            role: Role::Assistant,
            text: text.to_string(),
            images: 0,
            runs: md::render(text),
            kind: crate::proto::WireMsgKind::Plain,
        };
        let mut links = Vec::new();
        let lines = message_lines(&msg, 80, theme, false, &mut links);
        assert_eq!(links.len(), 1, "one link");
        let link = &links[0];
        assert_eq!(link.url, "https://example.com/x");
        assert_eq!(link.start, 4, "after 'see '");
        assert_eq!(link.end, 8, "the label is four characters");
        assert!(link.line < lines.len(), "the line it sits on exists");
    }
}
