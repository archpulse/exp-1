//! drawing the interface.

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
use crate::md;

use super::*;

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let suggestions = self.suggestions();
        // The frame about to be drawn replaces the clickable regions of the
        // one before it.
        self.clicks.borrow_mut().clear();

        // Height is budgeted top-down by hand. Letting the layout solver divide a
        // short terminal was what dropped the input band entirely once the fixed
        // rows no longer fitted.
        const FOOTER: u16 = 2;
        let footer = FOOTER.min(area.height);
        // A prompt written over several lines (shift+enter) takes the rows it
        // needs; the transcript gives them up.
        let typed_lines = self.thread().input.iter().filter(|ch| **ch == '\n').count() as u16;
        let attached = u16::from(!self.pending_images.is_empty());
        let input = (INPUT_HEIGHT + typed_lines + attached).min(area.height.saturating_sub(footer));
        let mut spare = area.height.saturating_sub(footer + input);

        // Keep at least one transcript row whenever there is room for one.
        let wants_overlay = if self.picker.is_some() {
            self.picker
                .as_ref()
                .map(Picker::height)
                .unwrap_or(0)
        } else {
            suggestions.len().min(POPUP_MAX) as u16
        };
        let overlay = wants_overlay.min(spare.saturating_sub(1));
        // A picker that renders nothing at all would leave the user stuck, so
        // give it a few rows even on a very short terminal.
        let overlay = if self.picker.is_some() && overlay < 3 {
            3.min(spare)
        } else {
            overlay
        };
        spare -= overlay;

        let wants_ask = self.asking.as_ref().map(AskView::height).unwrap_or(0);
        let ask = wants_ask.min(spare.saturating_sub(1));
        spare -= ask;

        let masthead = 2.min(spare);
        let transcript = spare - masthead;

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(masthead),
                Constraint::Length(transcript),
                Constraint::Length(overlay),
                Constraint::Length(ask),
                Constraint::Length(input),
                Constraint::Length(footer.min(1)),
                Constraint::Length(footer.saturating_sub(1)),
            ])
            .split(area);

        self.render_masthead(frame, rows[0]);
        self.render_transcript(frame, rows[1], rows[1].width as usize);
        if let Some(picker) = &self.picker {
            self.render_picker(frame, rows[2], picker);
        } else if overlay > 0 {
            self.render_popup(frame, rows[2], &suggestions);
        }
        if let Some(ask) = &self.asking {
            self.render_ask(frame, rows[3], ask);
        }
        self.render_input(frame, rows[4]);
        self.render_footer(frame, rows[5], rows[6], !suggestions.is_empty());
        self.capture_screen(frame);
    }

    /// The action under a screen cell, if the last frame drew one there.
    pub fn click_at(&self, column: u16, row: u16) -> Option<ClickAction> {
        self.clicks
            .borrow()
            .iter()
            .rev()
            .find(|target| {
                column >= target.area.x
                    && column < target.area.x + target.area.width
                    && row >= target.area.y
                    && row < target.area.y + target.area.height
            })
            .map(|target| target.action.clone())
    }

    /// Starts a selection at a screen position.
    pub fn begin_selection(&mut self, column: u16, row: u16) {
        self.selection = Some(((column, row), (column, row)));
    }

    /// Moves the loose end of the selection.
    pub fn extend_selection(&mut self, column: u16, row: u16) {
        if let Some((anchor, _)) = self.selection {
            self.selection = Some((anchor, (column, row)));
        }
    }

    /// The selection as (top-left, bottom-right), whatever way it was dragged.
    pub fn ordered_selection(&self) -> Option<((u16, u16), (u16, u16))> {
        let (a, b) = self.selection?;
        if (a.1, a.0) <= (b.1, b.0) {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    /// The selected text, taken from the screen as it was last drawn. Rows are
    /// trimmed at the end, because every row is padded to the full width.
    pub fn selected_text(&self) -> Option<String> {
        let ((x0, y0), (x1, y1)) = self.ordered_selection()?;
        let mut rows: Vec<String> = Vec::new();
        for y in y0..=y1 {
            let Some(row) = self.screen.get(y as usize) else {
                continue;
            };
            let chars: Vec<char> = row.chars().collect();
            let from = if y == y0 { x0 as usize } else { 0 }.min(chars.len());
            let to = if y == y1 {
                x1 as usize + 1
            } else {
                chars.len()
            }
            .min(chars.len())
            .max(from);
            rows.push(chars[from..to].iter().collect::<String>());
        }
        let text = rows.join("\n").trim_end().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Snapshots the drawn screen for copying, then paints the selection over
    /// it — the buffer is the one place the screen's text and its cells meet.
    fn capture_screen(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let selection = self.ordered_selection();
        let theme = self.theme();
        // A selection reads as a normal terminal one: a quiet tint behind the
        // text, not the loud colour the pickers use for a highlighted row.
        let highlight = Style::default().bg(theme.input_bg);
        let buf = frame.buffer_mut();
        self.screen = (area.y..area.y + area.height)
            .map(|y| {
                let mut line = String::new();
                for x in area.x..area.x + area.width {
                    if let Some(cell) = buf.cell((x, y)) {
                        line.push_str(cell.symbol());
                    }
                }
                line.trim_end().to_string()
            })
            .collect();
        let Some(((x0, y0), (x1, y1))) = selection else {
            return;
        };
        let last = area.x + area.width - 1;
        for y in y0..=y1 {
            // A row is only as long as its text: painting the padding past the
            // last character is what turned a selection into a solid block.
            let len = self
                .screen
                .get(y as usize)
                .map(|row| row.chars().count() as u16)
                .unwrap_or(0);
            if len == 0 {
                continue;
            }
            let row_end = area.x + len - 1;
            let (from, to) = if y == y0 && y == y1 {
                (x0, x1)
            } else if y == y0 {
                (x0, last)
            } else if y == y1 {
                (area.x, x1)
            } else {
                (area.x, last)
            };
            let to = to.min(row_end);
            let from = from.min(row_end);
            if from > to {
                continue;
            }
            for x in from..=to {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(highlight);
                }
            }
        }
    }

    /// The question panel: accent bar on the left, tabs, numbered options.
    fn render_ask(&self, frame: &mut Frame, area: Rect, ask: &AskView) {
        let theme = self.theme();
        let item = ask.current();

        let bar = |style: Style| {
            vec![
                Span::styled("▌", style),
                Span::styled(" ", Style::default().bg(theme.input_bg)),
            ]
        };
        let push = |lines: &mut Vec<Line<'static>>, runs: Vec<(Style, String)>| {
            let mut spans = bar(runs.first().map(|(style, _)| *style).unwrap_or_default());
            for (style, text) in runs {
                spans.push(Span::styled(
                    text,
                    Style::default().bg(theme.input_bg).patch(style),
                ));
            }
            lines.push(Line::from(spans));
        };

        let mut lines: Vec<Line<'static>> = Vec::new();
        let title = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(theme.dim);
        let chosen = Style::default().fg(theme.accent).add_modifier(Modifier::BOLD);

        push(&mut lines, vec![(title, "Questions".to_string())]);
        push(&mut lines, vec![(dim, String::new())]);

        if ask.tabs_visible() {
            let mut runs = Vec::new();
            for (position, question) in ask.questions.iter().enumerate() {
                if position > 0 {
                    runs.push((dim, "    ".to_string()));
                }
                let active = position == ask.index;
                runs.push((
                    if active { chosen } else { dim },
                    question.label.clone(),
                ));
            }
            runs.push((dim, format!("    {}", ask.submit_label)));
            push(&mut lines, runs);
            push(&mut lines, vec![(dim, String::new())]);
        }

        push(&mut lines, vec![(title, item.text.clone())]);
        push(&mut lines, vec![(dim, String::new())]);

        for (position, option) in item.options.iter().enumerate() {
            let active = position == ask.selected;
            push(
                &mut lines,
                vec![(
                    if active { chosen } else { Style::default() },
                    format!("{}. {}", position + 1, option),
                )],
            );
        }

        push(&mut lines, vec![(dim, String::new())]);
        let own = if ask.typed {
            Style::default().fg(theme.accent)
        } else {
            Style::default()
        };
        push(
            &mut lines,
            vec![(
                own,
                "Type your own answer".to_string(),
            )],
        );
        push(&mut lines, vec![(dim, String::new())]);
        push(
            &mut lines,
            vec![(
                dim,
                "⇥tab  ↑↓ select  enter confirm  esc dismiss".to_string(),
            )],
        );

        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_masthead(&self, frame: &mut Frame, area: Rect) {
        let theme = self.theme();
        let lines = vec![
            Line::from(vec![
                Span::styled(">_", Style::default().fg(theme.accent)),
                Span::raw(" "),
                Span::styled("exp-1 agent", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(" (v{})", env!("CARGO_PKG_VERSION")),
                    Style::default().fg(theme.dim),
                ),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(self.cwd.clone(), Style::default().fg(theme.dim)),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_transcript(&mut self, frame: &mut Frame, area: Rect, width: usize) {
        let theme = self.theme();
        let (theme_index, show_output) = (self.theme, self.show_tool_output);

        // Only what is missing is built: a long conversation must not be
        // re-wrapped on every frame.
        {
            let thread = self.thread_mut();
            thread.bake(width, theme_index, show_output, theme);
        }
        let thread = self.thread();

        let mut prefix: Vec<Line> = Vec::new();
        if thread.transcript.is_empty() {
            prefix.push(Line::raw(""));
            if self.first_run {
                // First run shows command names only; descriptions are available
                // through the completion popup and /help.
                prefix.push(Line::styled(
                    "  To get started, describe a task or try one of these commands:",
                    Style::default().fg(theme.dim),
                ));
                prefix.push(Line::raw(""));
                for (name, _) in COMMANDS {
                    prefix.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(name, Style::default().fg(theme.accent)),
                    ]));
                }
                for (name, _) in &self.skills {
                    prefix.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(name.clone(), Style::default().fg(theme.accent)),
                    ]));
                }
            } else {
                prefix.push(Line::styled(
                    "  Describe a task to start; / opens the commands, /help the keys.",
                    Style::default().fg(theme.dim),
                ));
            }
        }

        // The streamed text and the activity line are small, so they are built
        // fresh each frame.
        let mut tail: Vec<Line> = Vec::new();
        if !thread.live.is_empty() {
            // The text being streamed is markdown too, so it is formatted while
            // it arrives instead of snapping into shape once the turn ends.
            for run_line in md::render(&thread.live) {
                for part in wrap_runs(&run_line, width) {
                    let mut spans: Vec<Span> = Vec::new();
                    for run in &part {
                        if run.style == md::Style::CodeBlock {
                            spans.push(Span::styled("│ ", Style::default().fg(theme.dim)));
                        }
                        spans.push(Span::styled(run.text.clone(), md_style(run.style, theme)));
                    }
                    if spans.is_empty() {
                        spans.push(Span::raw(""));
                    }
                    tail.push(Line::from(spans));
                }
            }
        }
        // The activity line says what is happening now: thinking, preparing a
        // tool call, or running one. It sits under the streamed text when the
        // model has already written some — the arguments of a large tool call
        // are the quietest part of a turn, and this line is the only sign of
        // life there.
        if thread.busy() && (thread.live.is_empty() || thread.preparing.is_some()) {
            let label: &str = match &thread.preparing {
                Some(preparing) => preparing,
                None if thread.phase == Phase::Running => "running",
                None => "thinking",
            };
            let elapsed = thread
                .activity_since
                .map(|start| start.elapsed().as_secs())
                .unwrap_or(0);
            tail.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    SPINNER[self.tick % SPINNER.len()].to_string(),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    format!(" {label} {elapsed}s"),
                    Style::default().fg(theme.dim),
                ),
            ]));
        }

        let total =
            prefix.len() + thread.lines.iter().map(Vec::len).sum::<usize>() + tail.len();
        let height = area.height as usize;
        let max_scroll = total.saturating_sub(height);

        // A view scrolled up stays where it is while new lines arrive; one at
        // the bottom follows the conversation.
        {
            let thread = self.thread_mut();
            let grew = total.saturating_sub(thread.last_total);
            if grew > 0 && thread.scroll > 0 {
                thread.scroll = thread.scroll.saturating_add(grew as u16);
            }
            thread.last_total = total;
        }
        let thread = self.thread();

        // `scroll` counts lines scrolled up from the bottom, so the newest
        // message is what you see unless you wheel upwards.
        let scroll = (thread.scroll as usize).min(max_scroll);
        let offset = max_scroll - scroll;
        // Assembled line by line, so a link knows the row it was drawn on.
        let mut visible: Vec<Line> = Vec::with_capacity(height);
        let mut absolute = 0usize;
        let keep = |line: &Line<'static>, absolute: usize, visible: &mut Vec<Line>| {
            if absolute >= offset && absolute < offset + height {
                visible.push(line.clone());
            }
        };
        for line in prefix.iter() {
            keep(line, absolute, &mut visible);
            absolute += 1;
        }
        for (message, msg_lines) in thread.lines.iter().enumerate() {
            let first = absolute;
            for (line_index, line) in msg_lines.iter().enumerate() {
                keep(line, absolute, &mut visible);
                if absolute >= offset
                    && absolute < offset + height
                    && let Some(links) = thread.links.get(message)
                {
                    let row = area.y + (absolute - offset) as u16;
                    for link in links.iter().filter(|link| link.line == line_index) {
                        self.clicks.borrow_mut().push(ClickTarget {
                            area: Rect::new(
                                area.x + link.start as u16,
                                row,
                                link.end.saturating_sub(link.start) as u16,
                                1,
                            ),
                            action: ClickAction::Link(link.url.clone()),
                        });
                    }
                }
                absolute += 1;
            }
            // A click on a prompt goes back to it: everything after it — the
            // conversation and the file changes — is what a rewind drops.
            if thread.kind == ThreadKind::Main
                && let Some(msg) = thread.transcript.get(message)
                && msg.role == Role::User
            {
                let from = first.max(offset);
                let to = absolute.min(offset + height);
                if to > from {
                    self.clicks.borrow_mut().push(ClickTarget {
                        area: Rect::new(
                            area.x,
                            area.y + (from - offset) as u16,
                            area.width,
                            (to - from) as u16,
                        ),
                        action: ClickAction::Rewind {
                            index: message,
                            text: msg.text.clone(),
                        },
                    });
                }
            }
        }
        for line in tail.iter() {
            keep(line, absolute, &mut visible);
            absolute += 1;
        }
        frame.render_widget(Paragraph::new(visible), area);
    }

    fn render_popup(&self, frame: &mut Frame, area: Rect, suggestions: &[Suggestion]) {
        let theme = self.theme();
        let width = area.width as usize;
        let height = area.height as usize;
        let selected = self.thread().selected;
        // The label column follows the longest label, so a long command name
        // can never run into its own description.
        let name_column = suggestions
            .iter()
            .map(|suggestion| suggestion.label.chars().count())
            .max()
            .unwrap_or(NAME_COLUMN)
            .clamp(NAME_COLUMN, 30);
        // The list is often longer than the room the layout gives it, so the
        // window follows the highlight instead of always starting at the top.
        let start = if height == 0 || selected < height {
            0
        } else {
            selected + 1 - height
        };
        let mut lines: Vec<Line> = Vec::new();
        for (offset, suggestion) in suggestions.iter().skip(start).take(height.max(1)).enumerate() {
            let index = start + offset;
            self.clicks.borrow_mut().push(ClickTarget {
                area: Rect::new(area.x, area.y + offset as u16, area.width, 1),
                action: ClickAction::Suggestion(index),
            });
            let text: String = format!(
                "  {:<name_column$} {}",
                suggestion.label, suggestion.description
            )
            .chars()
            .take(width)
            .collect();
            let style = if index == selected {
                Style::default().bg(theme.selection).fg(rgb(255, 255, 255))
            } else {
                Style::default()
            };
            lines.push(Line::styled(pad_to(&text, width), style));
        }
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_picker(&self, frame: &mut Frame, area: Rect, picker: &Picker) {
        let theme = self.theme();
        let width = area.width as usize;
        let height = area.height as usize;

        // Spacers are a nicety; items are the point. On a short panel the
        // spacers go first so the choices stay visible.
        let compact = height < 8;
        let mut lines: Vec<Line> = Vec::new();
        if !compact {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            format!("  {}", picker.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        if !compact {
            lines.push(Line::raw(""));
        }

        // What is typed narrows the rows; each keeps its place in the full
        // list, which is what a pick means.
        let shown = self.shown_picker_items(picker);
        let total = shown.len();
        // One row for the hints, plus one for the range line when not compact.
        let tail = if compact { 1 } else { 2 };
        let capacity = height
            .saturating_sub(lines.len() + tail)
            .max(1);
        let visible = capacity.min(total);
        let selection = shown
            .iter()
            .position(|(index, _)| *index == picker.selected)
            .unwrap_or(0);
        let start = if total <= visible {
            0
        } else {
            // Centre the window on the selection so it cannot scroll out of sight.
            selection
                .saturating_sub(visible / 2)
                .min(total - visible)
        };

        let column = shown
            .iter()
            .map(|(_, item)| item.label.chars().count())
            .max()
            .unwrap_or(0)
            + 2;
        let items_top = lines.len();
        for (offset, (index, item)) in shown[start..start + visible].iter().enumerate() {
            let index = *index;
            self.clicks.borrow_mut().push(ClickTarget {
                area: Rect::new(area.x, area.y + (items_top + offset) as u16, area.width, 1),
                action: ClickAction::PickerRow(index),
            });
            let selected = index == picker.selected;
            let marker = if selected { ">" } else { " " };
            // Numbered as they are shown, not as they sit in the full list: a
            // filtered list that starts at 73 reads like a mistake.
            let body: String = format!(
                " {marker} {}. {:<column$}{}",
                start + offset + 1,
                item.label,
                item.description
            );
            let style = if selected {
                Style::default().bg(theme.selection).fg(rgb(255, 255, 255))
            } else {
                Style::default()
            };
            lines.push(Line::styled(pad_to(&body, width), style));
        }

        if !compact && total > visible {
            lines.push(Line::styled(
                format!(
                    "  ({}-{} of {total}{})",
                    start + 1,
                    start + visible,
                    if self.picker_filter.is_empty() {
                        String::new()
                    } else {
                        format!(", filter '{}'", self.picker_filter)
                    }
                ),
                Style::default().fg(theme.dim),
            ));
        } else if !compact && !self.picker_filter.is_empty() {
            lines.push(Line::styled(
                format!("  ({total} matching '{}')", self.picker_filter),
                Style::default().fg(theme.dim),
            ));
        }
        if total == 0 {
            lines.push(Line::styled(
                "  nothing matches — backspace to widen",
                Style::default().fg(theme.dim),
            ));
        }
        let hints = match picker.kind {
            PickerKind::Scoped => "type to filter · enter tick · ctrl-a all · esc save",
            PickerKind::Message => "enter pick · esc back",
            _ => "type to filter · enter select · esc back",
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(hints, Style::default().fg(theme.dim)),
        ]));

        let block = Block::default().style(Style::default().bg(theme.input_bg));
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }


    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let theme = self.theme();
        let thread = self.thread();
        let caret = Style::default().fg(theme.accent);

        // Typing stays live during a turn, so the caret keeps blinking.
        let show_caret = self.picker.is_none() && self.caret_visible();

        // The prompt can span lines, so it is laid out line by line with the
        // caret at the cursor's own line and column.
        let mut lines: Vec<Line> = vec![Line::raw("")];
        // An attached picture shows as a filled chip above the prompt, the way
        if !self.pending_images.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{} image{} queued", self.pending_images.len(), if self.pending_images.len() == 1 { "" } else { "s" }),
                    Style::default().bg(theme.accent).fg(theme.input_bg),
                ),
                Span::styled(
                    "  ctrl-v adds another · enter sends",
                    Style::default().fg(theme.dim),
                ),
            ]));
        }
        if thread.input.is_empty() {
            let mut spans = vec![Span::styled("› ", Style::default().fg(theme.dim))];
            if show_caret {
                spans.push(Span::styled(CARET, caret));
            }
            spans.push(Span::styled(
                if self.entering_key.is_some() {
                    "paste the API key"
                } else if self.asking.is_some() {
                    "your answer"
                } else {
                    thread.placeholder()
                },
                Style::default().fg(theme.dim),
            ));
            lines.push(Line::from(spans));
        } else {
            let mut spans = vec![Span::styled("› ", Style::default().fg(theme.dim))];
            for (index, ch) in thread.input.iter().enumerate() {
                if index == thread.cursor && show_caret {
                    spans.push(Span::styled(CARET, caret));
                }
                if *ch == '\n' {
                    lines.push(Line::from(std::mem::take(&mut spans)));
                    spans.push(Span::styled("  ", Style::default().fg(theme.dim)));
                    continue;
                }
                spans.push(Span::raw(ch.to_string()));
            }
            if thread.cursor >= thread.input.len() {
                if show_caret {
                    spans.push(Span::styled(CARET, caret));
                } else {
                    spans.push(Span::raw(" "));
                }
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::raw(""));

        let block = Block::default().style(Style::default().bg(theme.input_bg));
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn render_footer(&self, frame: &mut Frame, model_area: Rect, hint_area: Rect, popup_open: bool) {
        let theme = self.theme();
        let thread = self.thread();

        let mut model_spans = vec![
            Span::raw("  "),
            Span::styled(self.model_line(), Style::default().fg(theme.dim)),
            Span::styled(format!("  ·  {}", self.cwd), Style::default().fg(theme.dim)),
        ];
        if !thread.name.is_empty() {
            model_spans.push(Span::raw("  ·  "));
            model_spans.push(Span::styled(
                thread.name.clone(),
                Style::default().fg(theme.accent),
            ));
        }
        model_spans.push(Span::styled(
            format!("  ·  {}", thread.kind.label()),
            Style::default().fg(theme.dim),
        ));
        // Only worth showing when it is not the default: plan mode changes what
        // the agent is allowed to do.
        if self.mode == "plan" {
            model_spans.push(Span::styled(
                "  ·  plan mode",
                Style::default().fg(theme.warn),
            ));
        }
        if let Some(usage) = &thread.usage {
            model_spans.push(Span::styled(
                format!("  ·  {}", usage.render()),
                Style::default().fg(theme.dim),
            ));
        }
        // How much of the window is gone, so a long turn is never a surprise.
        if let Some((used, window)) = self.context_share() {
            model_spans.push(Span::styled(
                format!(
                    "  ·  {}% of {}k",
                    used.saturating_mul(100) / window.max(1),
                    window / 1000
                ),
                Style::default().fg(theme.dim),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(model_spans)), model_area);

        // A fresh copy note takes the right-hand slot for a moment.
        let note = self
            .copied
            .as_ref()
            .filter(|(_, at)| at.elapsed() < std::time::Duration::from_millis(2500));
        let right = match note {
            Some((text, _)) => text.as_str(),
            None => match thread.kind {
                ThreadKind::Main => "ctrl + / for side",
                ThreadKind::Side => "ctrl + / to switch · ctrl + c to close",
            },
        };
        let right_style = match (note, thread.kind) {
            (Some(_), _) => Style::default().fg(theme.accent),
            (None, ThreadKind::Main) => Style::default().fg(theme.dim),
            (None, ThreadKind::Side) => Style::default().fg(theme.warn),
        };

        let left_text = if let Some(error) = &thread.error {
            format!("{error} · ctrl-c to quit")
        } else if popup_open {
            let total = self.suggestions().len();
            let chosen = self.thread().selected.min(total.saturating_sub(1)) + 1;
            format!("↑↓ select {chosen}/{total} · tab complete · enter accept")
        } else if thread.busy() {
            "esc esc interrupts · /btw runs alongside".to_string()
        } else {
            "enter send · ctrl-c quit · ctrl-o output · ctrl-g bottom · ↑↓ history".to_string()
        };
        let left_style = if thread.error.is_some() {
            Style::default().fg(theme.warn)
        } else {
            Style::default().fg(theme.dim)
        };

        // Lay the line out to exactly the available width: two spaces of
        // leading indent, the left hint, a gap, then the right hint. Missing the
        // indent here is what clipped the right-hand text.
        const INDENT: usize = 2;
        let width = hint_area.width as usize;

        // The microphone button lives in the corner, past the thread hint: a
        // click starts dictation, a second click stops it.
        let mic = match self.dictating_since {
            Some(start) => format!("● mic {}s", start.elapsed().as_secs()),
            None => "mic".to_string(),
        };
        let mic_len = mic.chars().count();
        let mic_style = if self.dictating_since.is_some() {
            Style::default().fg(theme.warn)
        } else {
            Style::default().fg(theme.dim)
        };
        let separator = "  ·  ";
        let mut right = right.to_string();
        // The button is the last thing to give way when space runs out.
        let room = width.saturating_sub(INDENT + 1 + separator.chars().count() + mic_len);
        if right.chars().count() > room {
            right = right.chars().take(room).collect();
        }
        let right_len = right.chars().count() + separator.chars().count() + mic_len;

        let budget = width.saturating_sub(INDENT + 1 + right_len);
        let mut left_text: String = left_text.chars().take(budget).collect();
        // Never leave a dangling separator or half a word behind.
        if left_text.chars().count() == budget
            && let Some(cut) = left_text.rfind(" · ")
        {
            left_text.truncate(cut);
        }
        // Too short to say anything useful; a clean gap beats a fragment.
        if left_text.chars().count() < 12 {
            left_text.clear();
        }
        let gap = width.saturating_sub(INDENT + left_text.chars().count() + right_len);

        let right_column = INDENT + left_text.chars().count() + gap;
        let mut line: Vec<Span> = vec![Span::raw(" ".repeat(INDENT))];
        line.push(Span::styled(left_text, left_style));
        line.push(Span::raw(" ".repeat(gap)));
        line.push(Span::styled(right, right_style));
        line.push(Span::styled(separator, Style::default().fg(theme.dim)));
        line.push(Span::styled(mic, mic_style));
        frame.render_widget(Paragraph::new(Line::from(line)), hint_area);
        // Both ends of that line are buttons.
        self.clicks.borrow_mut().push(ClickTarget {
            area: Rect::new(
                hint_area.x + (right_column + right_len - mic_len) as u16,
                hint_area.y,
                mic_len as u16,
                1,
            ),
            action: ClickAction::Mic,
        });
        if note.is_none() {
            self.clicks.borrow_mut().push(ClickTarget {
                area: Rect::new(
                    hint_area.x + right_column as u16,
                    hint_area.y,
                    (right_len - separator.chars().count() - mic_len) as u16,
                    1,
                ),
                action: ClickAction::SideThread,
            });
        }
    }
}

/// Maps a parsed run to concrete colours for the active theme.
fn md_style(role: md::Style, theme: &Theme) -> Style {
    match role {
        md::Style::Text => Style::default(),
        md::Style::Heading => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        md::Style::Strong => Style::default().add_modifier(Modifier::BOLD),
        md::Style::Emphasis => Style::default().add_modifier(Modifier::ITALIC),
        md::Style::Code => Style::default().fg(theme.code),
        md::Style::CodeBlock => Style::default().fg(theme.code),
        md::Style::Quote => Style::default().fg(theme.dim),
        md::Style::Bullet => Style::default().fg(theme.accent),
        md::Style::Rule => Style::default().fg(theme.dim),
        md::Style::Link => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::UNDERLINED),
        md::Style::Math => Style::default().fg(theme.math),
    }
}

fn pad_to(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len >= width {
        text.chars().take(width).collect()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

/// The lines one transcript message renders to. Tool output is skipped unless
/// the reader asked for it.
pub(crate) fn message_lines(
    msg: &Msg,
    width: usize,
    theme: &'static Theme,
    show_output: bool,
    links: &mut Vec<LinkSpan>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match msg.role {
        Role::User => {
            if msg.images > 0 {
                lines.push(pad_line(
                    &format!(
                        "  ▣ {} image{} attached",
                        msg.images,
                        if msg.images == 1 { "" } else { "s" }
                    ),
                    width,
                    theme.input_bg,
                ));
            }
            let mut first = true;
            for body in msg.text.split('\n') {
                let room = width.saturating_sub(2);
                for wrapped in wrap_text(body, room) {
                    let lead = if first { "› " } else { "  " };
                    first = false;
                    lines.push(pad_line(&format!("{lead}{wrapped}"), width, theme.input_bg));
                }
            }
        }
        Role::System => {
            for (index, body) in msg.text.split('\n').enumerate() {
                let lead = if index == 0 { "• " } else { "  " };
                let room = width.saturating_sub(lead.chars().count());
                for (part, wrapped) in wrap_text(body, room).into_iter().enumerate() {
                    let prefix = if index == 0 && part == 0 { "• " } else { "  " };
                    lines.push(Line::styled(
                        format!("{prefix}{wrapped}"),
                        Style::default().fg(theme.dim),
                    ));
                }
            }
        }
        Role::Tool => {
            // The call itself is the line that matters; its output is noise
            // unless the reader asked for it.
            if msg.kind == crate::proto::WireMsgKind::ToolOutput && !show_output {
                return lines;
            }
            for body in msg.text.split('\n') {
                let room = width.saturating_sub(4);
                for wrapped in wrap_text(body, room) {
                    lines.push(Line::styled(
                        format!("    {wrapped}"),
                        Style::default().fg(theme.dim),
                    ));
                }
            }
        }
        Role::Assistant => {
            if msg.runs.is_empty() {
                for body in msg.text.split('\n') {
                    lines.push(Line::raw(body.to_string()));
                }
            } else {
                for run_line in &msg.runs {
                    for part in wrap_runs(run_line, width) {
                        let mut spans: Vec<Span> = Vec::new();
                        let line_index = lines.len();
                        let mut column = 0usize;
                        for run in &part {
                            if run.style == md::Style::CodeBlock {
                                spans.push(Span::styled("│ ", Style::default().fg(theme.dim)));
                                column += 2;
                            }
                            // The column advances for every run, so a link
                            // knows where it starts even when text comes first.
                            let width = run.text.chars().count();
                            if let Some(url) = &run.url {
                                links.push(LinkSpan {
                                    line: line_index,
                                    start: column,
                                    end: column + width,
                                    url: url.clone(),
                                });
                            }
                            column += width;
                            spans.push(Span::styled(
                                run.text.clone(),
                                md_style(run.style, theme),
                            ));
                        }
                        if spans.is_empty() {
                            spans.push(Span::raw(""));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
        }
    }
    lines.push(Line::raw(""));
    lines
}

fn pad_line(text: &str, width: usize, bg: Color) -> Line<'static> {
    Line::styled(pad_to(text, width), Style::default().bg(bg))
}
