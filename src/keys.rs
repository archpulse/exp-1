//! what every key does.

use crossterm::event::{
    KeyCode, KeyEvent,
    KeyModifiers,
};
use std::time::{ Instant };

use super::*;

/// Deletes the run of whitespace before the cursor, then the word before it.
fn delete_word_back(input: &mut Vec<char>, cursor: &mut usize) {
    while *cursor > 0 && input[*cursor - 1].is_whitespace() {
        input.remove(*cursor - 1);
        *cursor -= 1;
    }
    while *cursor > 0 && !input[*cursor - 1].is_whitespace() {
        input.remove(*cursor - 1);
        *cursor -= 1;
    }
}

/// Deletes the whitespace after the cursor, then the word after it.
fn delete_word_forward(input: &mut Vec<char>, cursor: usize) {
    while cursor < input.len() && input[cursor].is_whitespace() {
        input.remove(cursor);
    }
    while cursor < input.len() && !input[cursor].is_whitespace() {
        input.remove(cursor);
    }
}

fn handle_ask_key(app: &mut App, key: KeyEvent, ctx: &Ctx) {
    match key.code {
        KeyCode::Up => {
            if let Some(ask) = app.asking.as_mut() {
                ask.move_selection(-1);
                ask.typed = false;
            }
        }
        KeyCode::Down => {
            if let Some(ask) = app.asking.as_mut() {
                ask.move_selection(1);
                ask.typed = false;
            }
        }
        KeyCode::Tab => {
            if let Some(ask) = app.asking.as_mut()
                && ask.questions.len() > 1
            {
                ask.index = (ask.index + 1) % ask.questions.len();
                ask.selected = 0;
                ask.typed = false;
                app.thread_mut().clear_input();
            }
        }
        KeyCode::Enter => {
            let typed = app.thread().input_text();
            let answers = {
                let Some(ask) = app.asking.as_ref() else {
                    return;
                };
                ask.questions
                    .iter()
                    .enumerate()
                    .map(|(position, question)| {
                        if position == ask.index {
                            ask.answer_for_current(&typed)
                        } else {
                            question.options.first().cloned().unwrap_or_default()
                        }
                    })
                    .collect::<Vec<String>>()
            };
            let conn = ctx.conn.clone();
            let session = ctx.session(app.active);
            tokio::spawn(async move {
                let _ = conn.answer(&session, answers).await;
            });
            app.asking = None;
            app.thread_mut().clear_input();
        }
        KeyCode::Esc => {
            app.asking = None;
            app.thread_mut().clear_input();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit = true;
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let thread = app.thread_mut();
            thread.input.clear();
            thread.cursor = 0;
            if let Some(ask) = app.asking.as_mut() {
                ask.typed = false;
            }
        }
        KeyCode::Char(ch) => {
            let thread = app.thread_mut();
            thread.input.insert(thread.cursor, ch);
            thread.cursor += 1;
            if let Some(ask) = app.asking.as_mut() {
                ask.typed = true;
            }
        }
        KeyCode::Backspace => {
            let thread = app.thread_mut();
            if thread.cursor > 0 {
                thread.input.remove(thread.cursor - 1);
                thread.cursor -= 1;
            }
            let empty = thread.input.is_empty();
            if let Some(ask) = app.asking.as_mut() {
                ask.typed = !empty;
            }
        }
        _ => {}
    }
}

/// Word-wise deletion, shared by the prompt and the question panel.
///
/// Terminals report what the user calls Ctrl+Backspace as the single byte
/// `0x08`, which is also Ctrl+H, so that spelling has to be accepted too.
/// Alt+Backspace is the classic readline binding for the same operation.
fn delete_word(app: &mut App, key: KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let backward = match key.code {
        KeyCode::Char('h') | KeyCode::Char('w') => control,
        KeyCode::Backspace => {
            key.modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        }
        _ => false,
    };
    let forward = matches!(key.code, KeyCode::Delete) && control;
    if !backward && !forward {
        return false;
    }

    let thread = app.thread_mut();
    if backward {
        let cursor = &mut thread.cursor;
        delete_word_back(&mut thread.input, cursor);
    } else {
        delete_word_forward(&mut thread.input, thread.cursor);
    }
    let empty = thread.input.is_empty();
    if let Some(ask) = app.asking.as_mut() {
        ask.typed = !empty;
    }
    true
}

pub(crate) fn handle_key(app: &mut App, key: KeyEvent, ctx: &Ctx) {
    // ctrl+/ switches between the main conversation and the side thread.
    // shift+tab switches between building and planning: plan mode refuses the
    // tools that change files, so the agent proposes instead of editing.
    if key.code == KeyCode::BackTab {
        let next = if app.mode == "plan" { "build" } else { "plan" };
        app.mode = next.to_string();
        let conn = ctx.conn.clone();
        let tx = ctx.events.clone();
        let session = ctx.session(0);
        let wanted = next.to_string();
        tokio::spawn(async move {
            match conn.mode(&session, &wanted).await {
                Ok(note) => {
                    let _ = tx.send(AgentMsg::Done(0, note));
                }
                Err(err) => {
                    let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                }
            }
        });
        return;
    }

    // A terminal sends raw 0x1F, which crossterm reports as Ctrl+7; a kitty
    // that negotiated the CSI-u protocol reports Ctrl+/ instead.
    let ctrl_slash = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('/') | KeyCode::Char('7'));
    if ctrl_slash {
        app.active = 1 - app.active;
        app.picker = None;
        return;
    }

    // ctrl+shift+a cycles the permission mode: which tools ask before they
    // run. A terminal speaking the kitty keyboard protocol (kitty itself, foot,
    // wezterm, ghostty) sends exactly one CSI-u event for the chord, which
    // crossterm hands over as `a` carrying CONTROL+SHIFT; that is the single
    // spelling matched here. A terminal without the protocol never sends the
    // SHIFT, so plain ctrl+a arrives as `a` with CONTROL alone and keeps its
    // own meaning (it clears a model scope in the picker). Handled this early
    // so the chord works from anywhere, pickers and questions included.
    let permissions_key = key.code == KeyCode::Char('a')
        && key.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    if permissions_key {
        let next = app.permissions.cycle();
        app.permissions = next;
        // A snapshot fetched before this press may land after it; the epoch
        // keeps that older mode from being put back on screen.
        PERMISSIONS_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let conn = ctx.conn.clone();
        let tx = ctx.events.clone();
        let session = ctx.session(0);
        tokio::spawn(async move {
            // The server records the mode and writes its own note into the
            // transcript. The footer already shows the choice, so success is
            // not reported back through the turn-finished channel.
            if let Err(err) = conn.permissions(&session, next).await {
                let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
            }
        });
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => {
                app.quit = true;
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                let thread = app.thread_mut();
                thread.input.clear();
                thread.cursor = 0;
                return;
            }
            _ => {}
        }
    }

    if delete_word(app, key) {
        return;
    }

    // While a question is open the keyboard answers it, nothing else.
    if app.asking.is_some() {
        handle_ask_key(app, key, ctx);
        return;
    }

    if app.picker.is_some() {
        match key.code {
            KeyCode::Up => app.move_picker_selection(-1),
            KeyCode::Down => app.move_picker_selection(1),
            KeyCode::Backspace => {
                if !app.picker_filter.is_empty() {
                    app.picker_filter.pop();
                    app.select_first_shown_row();
                }
            }
            KeyCode::Enter => {
                let scoped = app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
                if scoped {
                    let index = app.picker.as_ref().map(|p| p.selected).unwrap_or(0);
                    toggle_scoped(app, index);
                } else {
                    select_from_picker(app, ctx);
                }
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // In the scope picker, ctrl-a puts every model back on the
                // shelf: an empty scope means no restriction at all. Plain
                // letters belong to the filter, so it needs the modifier.
                let scoped = app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
                if scoped {
                    app.scoped.clear();
                    app.open_scoped_picker();
                }
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                // Typing narrows a long list; the picker has nothing else to do
                // with letters.
                app.picker_filter.push(ch);
                app.select_first_shown_row();
            }
            KeyCode::Esc => {
                // Esc widens first: closing a picker with a filter still in it
                // would look like it lost the list.
                if !app.picker_filter.is_empty() {
                    app.picker_filter.clear();
                    app.select_first_shown_row();
                    return;
                }
                // A message menu is answered, not saved: closing it forgets
                // which message it was about.
                if app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Message)
                {
                    app.message_target = None;
                    app.picker = None;
                    return;
                }
                let scoped = app
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
                if scoped {
                    let chosen: Vec<String> = app.scoped.iter().cloned().collect();
                    let conn = ctx.conn.clone();
                    tokio::spawn(async move {
                        let _ = conn.scope_models(chosen).await;
                    });
                    let count = app.scoped.len();
                    app.picker = None;
                    app.thread_mut().push(
                        Role::System,
                        if count == 0 {
                            "All models are offered again".to_string()
                        } else {
                            format!("{count} model(s) will be offered in /model")
                        },
                    );
                } else {
                    app.picker = None;
                }
            }
            _ => {}
        }
        return;
    }

    let popup_len = app.suggestions().len();
    let busy = app.thread().busy();

    match key.code {
        KeyCode::Esc => {
            // A single Esc deliberately does nothing. A second one, inside the
            // window, aborts the running turn or closes the completion list.
            let now = Instant::now();
            let double = app
                .last_esc
                .is_some_and(|previous| now.duration_since(previous) < DOUBLE_ESC);
            app.last_esc = if double { None } else { Some(now) };
            if !double {
                return;
            }
            if !app.suggestions().is_empty() {
                app.thread_mut().popup_open = false;
            } else if app.thread().busy() {
                let conn = ctx.conn.clone();
                let index = app.active;
                let session = ctx.session(index);
                tokio::spawn(async move {
                    let _ = conn.interrupt(&session).await;
                });
            }
        }
        KeyCode::Up => {
            if popup_len > 0 {
                app.thread_mut().selected = app.thread().selected.saturating_sub(1);
            } else {
                app.thread_mut().history_prev();
            }
        }
        KeyCode::Down => {
            if popup_len > 0 {
                let last = popup_len - 1;
                app.thread_mut().selected = (app.thread().selected + 1).min(last);
            } else {
                app.thread_mut().history_next();
            }
        }
        KeyCode::Tab => {
            app.accept_suggestion();
        }
        KeyCode::Enter => {
            // Shift+Enter writes a line break instead of sending: a prompt can
            // be several lines, the way Codex takes it.
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                let thread = app.thread_mut();
                thread.input.insert(thread.cursor, '\n');
                thread.cursor += 1;
                return;
            }
            if popup_len > 0 {
                let typed = app.thread().input_text();
                let exact = app.suggestions().iter().any(|s| s.label == typed.trim());
                if !exact && app.accept_suggestion() {
                    return;
                }
                app.thread_mut().popup_open = false;
            }
            let text = app.thread().input_text();
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }

            // `/btw` is the one thing allowed while the main turn is busy.
            if let Some(question) = text.strip_prefix("/btw ") {
                let question = question.trim().to_string();
                if question.is_empty() {
                    app.thread_mut().push(Role::System, "usage: /btw <question>");
                } else {
                    start_side_question(app, ctx, question);
                }
                app.thread_mut().clear_input();
                return;
            }

            if busy {
                return;
            }
            if app.is_bare_command("/model") {
                app.open_model_picker();
                app.thread_mut().clear_input();
                return;
            }
            if app.is_bare_command("/themes") {
                app.open_theme_picker();
                app.thread_mut().clear_input();
                return;
            }
            if text.starts_with('/') && !text.contains(' ') {
                if !run_slash(app, ctx, &text) {
                    app.thread_mut()
                        .push(Role::System, format!("unknown command '{text}' — try /help"));
                }
                app.thread_mut().clear_input();
                return;
            }
            if text.starts_with('/') && run_slash(app, ctx, &text) {
                app.thread_mut().clear_input();
                return;
            }

            // A key being typed is sent to /login, not to the model. The text
            // is never empty here: an empty Enter returns above, which keeps
            // the prompt open until something is typed.
            if let Some((id, _)) = app.entering_key.clone() {
                let key = text.clone();
                app.thread_mut().clear_input();
                app.entering_key = None;
                let conn = ctx.conn.clone();
                let tx = ctx.events.clone();
                let session = ctx.session(0);
                tokio::spawn(async move {
                    match conn.login(&id, Some(key), Some(session), true).await {
                        Ok(config) => {
                            let _ = tx.send(AgentMsg::ProviderChanged(
                                config.label,
                                config.model,
                                config.effort,
                                config.message,
                            ));
                        }
                        Err(err) => {
                            let _ = tx.send(AgentMsg::Error(0, format!("login failed: {err:#}")));
                        }
                    }
                });
                return;
            }

            // Sending is now a request to the server, not a local turn.
            let images = std::mem::take(&mut app.pending_images)
                .into_iter()
                .map(|(media_type, data)| proto::WireImage { media_type, data })
                .collect::<Vec<_>>();
            let image_count = images.len();
            let index = app.active;
            let thread = app.thread_mut();
            thread.remember(&text);
            if thread.name.is_empty() {
                thread.name = thread_name(&text);
            }
            thread.push_user(&text, image_count);
            thread.clear_input();
            thread.error = None;
            thread.scroll = 0;
            thread.started_at = Some(Instant::now());
            thread.activity_since = Some(Instant::now());
            thread.phase = Phase::Streaming;
            let conn = ctx.conn.clone();
            let session = ctx.session(index);
            // `@file` mentions travel as paths; the server decides what to
            // attach and what to hand over to be read.
            let files = mentions(&text, &ctx.cwd());
            tokio::spawn(async move {
                if let Err(err) = conn.prompt(&session, &text, images, files).await {
                    eprintln!("exp-1: prompt failed: {err:#}");
                }
            });
        }
        KeyCode::Backspace => {
            let thread = app.thread_mut();
            if thread.cursor > 0 {
                thread.input.remove(thread.cursor - 1);
                thread.cursor -= 1;
                thread.selected = 0;
                thread.popup_open = true;
            }
        }
        KeyCode::Delete => {
            let thread = app.thread_mut();
            if thread.cursor < thread.input.len() {
                thread.input.remove(thread.cursor);
                thread.selected = 0;
                thread.popup_open = true;
            }
        }
        KeyCode::Left => {
            let thread = app.thread_mut();
            thread.cursor = thread.cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            let thread = app.thread_mut();
            thread.cursor = (thread.cursor + 1).min(thread.input.len());
        }
        KeyCode::Home => app.thread_mut().cursor = 0,
        KeyCode::End => {
            let thread = app.thread_mut();
            thread.cursor = thread.input.len();
        }
        KeyCode::PageUp => {
            let thread = app.thread_mut();
            thread.scroll = thread.scroll.saturating_add(10);
        }
        KeyCode::PageDown => {
            let thread = app.thread_mut();
            thread.scroll = thread.scroll.saturating_sub(10);
        }
        KeyCode::Char(ch) => {
            // ctrl+v takes a picture off the clipboard instead of typing a "v".
            if ch == 'v' && key.modifiers.contains(KeyModifiers::CONTROL) {
                attach_clipboard_image(app);
                return;
            }
            // ctrl+o shows the output of tool calls, which is hidden by default:
            // the call lines are the story, the output is the footnote.
            if ch == 'o' && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.show_tool_output = !app.show_tool_output;
                return;
            }
            // ctrl+r is the microphone: the first press starts dictation, the
            // second stops it.
            if ch == 'r' && key.modifiers.contains(KeyModifiers::CONTROL) {
                toggle_dictation(app, ctx);
                return;
            }
            // ctrl+g goes back to the newest message, ctrl+t to the oldest:
            // paging through a long conversation should not be the only way.
            if ch == 'g' && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.thread_mut().scroll = 0;
                return;
            }
            if ch == 't' && key.modifiers.contains(KeyModifiers::CONTROL) {
                // Clamped to the transcript's own length when it is drawn.
                app.thread_mut().scroll = u16::MAX;
                return;
            }
            // Typing stays available while a turn runs, so a /btw side question
            // can be asked mid-task.
            let thread = app.thread_mut();
            thread.input.insert(thread.cursor, ch);
            thread.cursor += 1;
            thread.selected = 0;
            thread.popup_open = true;
            thread.history_index = None;
        }
        _ => {}
    }
}

/// A click that did not drag: do what the thing under it does. A picker row is
/// Enter on that row, a suggestion is Tab, the footer hint is `ctrl + /`, and a
/// link opens in the browser.
pub(crate) fn click_at(app: &mut App, ctx: &Ctx, column: u16, row: u16) {
    let Some(action) = app.click_at(column, row) else {
        return;
    };
    match action {
        app::ClickAction::PickerRow(index) => {
            if let Some(picker) = app.picker.as_mut() {
                picker.selected = index;
            }
            let scoped = app
                .picker
                .as_ref()
                .is_some_and(|picker| picker.kind == app::PickerKind::Scoped);
            if scoped {
                toggle_scoped(app, index);
            } else {
                select_from_picker(app, ctx);
            }
        }
        app::ClickAction::Suggestion(index) => {
            app.thread_mut().selected = index;
            app.accept_suggestion();
        }
        app::ClickAction::SideThread => {
            app.active = 1 - app.active;
            app.picker = None;
        }
        app::ClickAction::Link(url) => crate::oauth::open_in_browser(&url),
        app::ClickAction::Mic => toggle_dictation(app, ctx),
        app::ClickAction::Rewind { index, text } => {
            // Not straight to reverting: a click asks what to do with it.
            app.open_message_picker(index, text);
        }
    }
}

/// Goes back to a message: the conversation, what the model was told after it
/// and the file changes those turns made.
pub(crate) fn revert_to(app: &mut App, ctx: &Ctx, index: usize, text: String) {
    let conn = ctx.conn.clone();
    let session = ctx.session(0);
    let tx = ctx.events.clone();
    app.threads[0].error = None;
    tokio::spawn(async move {
        match conn.rewind(&session, index, &text).await {
            Ok(_) => {
                // The transcript is the server's now, note and all.
                let epoch = PERMISSIONS_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
                if let Ok(snapshot) = conn.snapshot(&session).await {
                    let _ = tx.send(AgentMsg::Loaded(0, snapshot, epoch));
                }
                let _ = tx.send(AgentMsg::Rewound(text));
            }
            Err(err) => {
                let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
            }
        }
    });
}

/// The microphone button: the first press starts the recognizer, the second
/// stops it and lets it flush the words still in flight.
fn toggle_dictation(app: &mut App, ctx: &Ctx) {
    if let Some(dictation) = app.dictation.take() {
        app.dictating_since = None;
        // Stopping waits for the recognizer to finish, so it does not block the
        // interface.
        tokio::spawn(async move { dictation.stop() });
        return;
    }
    let tx = ctx.events.clone();
    let ready = tx.clone();
    let failed = tx.clone();
    let partial = tx.clone();
    match crate::dictate::Dictation::start(
        move |text| {
            let _ = tx.send(AgentMsg::Dictation(text));
        },
        move |complaint| {
            let _ = failed.send(AgentMsg::DictationFailed(complaint));
        },
        move || {
            let _ = ready.send(AgentMsg::DictationStopped);
        },
        move |pending| {
            let _ = partial.send(AgentMsg::Dictating(pending));
        },
    ) {
        Ok(dictation) => {
            app.dictation = Some(dictation);
            app.dictating_since = Some(Instant::now());
        }
        Err(err) => {
            // The transcript is the server's; a local note would put the two
            // out of step, so failures go in the footer's error slot instead.
            app.threads[0].error = Some(format!("dictation did not start: {err:#}"));
        }
    }
}
