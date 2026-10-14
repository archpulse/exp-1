//! the slash commands and the pickers they open.

use std::path::PathBuf;
use std::time::{ Instant };

use super::*;

/// One turn: request -> stream -> run tools -> repeat, until the model stops
/// asking for tools. Runs as its own task so a double Esc can abort it.
pub(crate) fn run_slash(app: &mut App, ctx: &Ctx, line: &str) -> bool {
    let (name, rest) = match line.split_once(' ') {
        Some((name, rest)) => (name, rest.trim()),
        None => (line, ""),
    };

    match name {
        "/help" => {
            app.thread_mut().push(
                Role::System,
                "enter send · ctrl-c quit · ctrl-o tool output · ctrl-v paste image · ctrl-backspace deletes a word · esc esc interrupts\nctrl-/ switches thread · ctrl-g jumps to the newest message · ctrl-t to the oldest · pgup/pgdn or the wheel scroll",
            );
        }
        "/cd" => {
            // With no argument it reports where the session is, which is the
            // question people actually have when they type it.
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            let wanted = rest.to_string();
            let cwd = ctx.cwd.clone();
            tokio::spawn(async move {
                match conn.cd(&session, &wanted).await {
                    Ok(landed) => {
                        if !landed.is_empty()
                            && let Ok(mut slot) = cwd.lock()
                        {
                            *slot = PathBuf::from(&landed);
                        }
                        let _ = tx.send(AgentMsg::Cwd(landed));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/effort" => {
            // The list belongs to the current model, so it is read fresh.
            if app.efforts_for(&app.model).is_empty() {
                app.thread_mut()
                    .push(Role::System, "this model takes no reasoning effort");
            } else {
                app.open_effort_picker("Select Effort");
            }
            app.thread_mut().clear_input();
            return true;
        }
        "/mcp" => {
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.mcp().await {
                    Ok(servers) => {
                        let _ = tx.send(AgentMsg::Done(0, render_mcp(&servers)));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/fork" => {
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            let session = ctx.session(0);
            let sessions = ctx.sessions.clone();
            tokio::spawn(async move {
                match conn.fork(&session).await {
                    Ok(fresh) => {
                        if let Ok(mut slots) = sessions.lock() {
                            slots[0] = fresh.clone();
                        }
                        let _ = tx.send(AgentMsg::Attached(0, "fork".to_string()));
                        match conn.stream(&fresh).await {
                            Ok(events) => {
                                if let Ok(snapshot) = conn.snapshot(&fresh).await {
                                    let _ = tx.send(AgentMsg::Loaded(0, snapshot));
                                }
                                pump_events(0, events, tx.clone());
                                let _ = tx.send(AgentMsg::Done(
                                    0,
                                    "forked: this conversation continues as a new session".to_string(),
                                ));
                            }
                            Err(err) => {
                                let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                            }
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/undo" => {
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.undo(&session).await {
                    Ok(note) => {
                        let _ = tx.send(AgentMsg::Done(0, note));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/diff" => {
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.diff(&session).await {
                    Ok(diff) => {
                        let _ = tx.send(AgentMsg::Done(0, diff));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/compact" => {
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.compact(&session).await {
                    Ok(()) => {
                        let _ = tx.send(AgentMsg::Done(
                            0,
                            "compacted; the summary is in the conversation".to_string(),
                        ));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/retitle" => {
            // The new name arrives as a Title event, so nothing is shown here
            // beyond the fact that the model was asked.
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                if let Err(err) = conn.retitle(&session).await {
                    let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                }
            });
            app.thread_mut()
                .push(Role::System, "naming this session again from the conversation…");
            app.thread_mut().clear_input();
            return true;
        }
        "/clear" => {
            // The screen is the visible half of a conversation; the other half
            // is the history the server sends the model, so both are emptied.
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            let sessions = [ctx.session(0), ctx.session(1)];
            tokio::spawn(async move {
                for session in sessions {
                    if let Err(err) = conn.clear(&session).await {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            let thread = app.thread_mut();
            thread.clear_transcript();
            thread.reasoning.clear();
            thread.usage = None;
            thread.error = None;
            thread.history.lock().unwrap().clear();
            // Clearing starts a brand new conversation rather than reusing the file.
            app.session_id = None;
            app.session_title.clear();
            app.title_started = false;
            // The footer shows the thread's name, so both threads lose theirs:
            // a cleared conversation is not "Анализ проекта" any more.
            app.threads[0].name.clear();
            app.threads[1].clear_transcript();
            app.threads[1].history.lock().unwrap().clear();
            app.threads[1].name.clear();
            app.thread_mut()
                .push(Role::System, "cleared; this conversation starts over");
        }
        "/status" => {
            let plan = app
                .plan
                .as_ref()
                .map(|p| format!("\nplan: {p}"))
                .unwrap_or_default();
            let theme_name = app.theme().name.to_string();
            let thread_label = app.thread().kind.label().to_string();
            let body = format!(
                "model: {}\neffort: {}\nworking directory: {}\ntools: read, write, edit, shell, ast\ntheme: {}\nthread: {}\nauth: Codex (ChatGPT OAuth){plan}",
                app.model,
                app.effort,
                app.cwd,
                theme_name,
                thread_label,
            );
            app.thread_mut().push(Role::System, body);
        }
        "/themes" => {
            if rest.is_empty() {
                app.open_theme_picker();
                app.thread_mut().clear_input();
                return true;
            }
            match THEMES
                .iter()
                .position(|theme| theme.name.eq_ignore_ascii_case(rest))
            {
                Some(index) => {
                    app.theme = index;
                    let name = app.theme().name.to_string();
                    app.thread_mut().push(Role::System, format!("Theme set to {name}"));
                }
                None => app
                    .thread_mut()
                    .push(Role::System, format!("unknown theme '{rest}' — try /themes")),
            }
        }
        "/model" => {
            if rest.is_empty() {
                app.open_model_picker();
                app.thread_mut().clear_input();
                return true;
            }
            let mut parts = rest.split_whitespace();
            let chosen = parts.next().unwrap_or_default().to_string();
            // `provider/slug` names one exactly; a bare name takes the provider
            // already in use when several offer it.
            let offered = app.offered_models();
            let row = match chosen.rsplit_once('/') {
                Some((provider, slug)) => offered
                    .iter()
                    .find(|model| model.provider == provider && model.slug == slug)
                    .cloned(),
                None => {
                    let active = app.active_provider_id().map(str::to_string);
                    offered
                        .iter()
                        .filter(|model| model.slug == chosen)
                        .min_by_key(|model| Some(model.provider.clone()) != active)
                        .cloned()
                }
            };
            let Some(row) = row else {
                app.thread_mut()
                    .push(Role::System, format!("unknown model '{chosen}' — try /model"));
                return true;
            };
            let levels: Vec<String> = row.efforts.iter().map(|e| e.effort.clone()).collect();
            if let Some(level) = parts.next() {
                // Providers that do not publish their levels take anything.
                if !levels.is_empty() && !levels.iter().any(|name| name == level) {
                    app.thread_mut().push(
                        Role::System,
                        format!(
                            "'{level}' is not a {} level; this model takes {}",
                            row.display_name,
                            levels.join(", ")
                        ),
                    );
                    return true;
                }
                app.effort = level.to_string();
            }
            app.model = row.slug.clone();
            // Switching models without naming an effort still has to land on an
            // effort the new model accepts.
            let settled = app::settle_effort(&levels, row.default_effort.as_deref(), &app.effort);
            app.effort = settled.clone();
            // A model from another provider means switching to it first.
            let switch = (!row.provider.is_empty()
                && app
                    .active_provider_id()
                    .is_none_or(|active| active != row.provider))
            .then(|| row.provider.clone());
            // The server owns the real values; keep the UI's copy in step.
            let conn = ctx.conn.clone();
            let model = row.slug.clone();
            let effort = settled.clone();
            tokio::spawn(async move {
                if let Some(provider) = switch {
                    let _ = conn.login(&provider, None, None, false).await;
                }
                let _ = conn.configure(Some(model), Some(effort)).await;
            });
            let (model, effort) = (app.model_label(), app.effort.clone());
            app.thread_mut()
                .push(Role::System, format!("Model changed to {model} {effort}"));
        }
        "/btw" => {
            if rest.is_empty() {
                app.thread_mut()
                    .push(Role::System, "usage: /btw <question>");
                return true;
            }
            start_side_question(app, ctx, rest.to_string());
        }
        "/login" => {
            // `/login <id>` signs in again; bare `/login` opens the picker.
            if !rest.is_empty() {
                let wanted = rest.to_string();
                // A key provider has nothing to force: it needs the key typed.
                // A device-login one re-runs the vendor's page instead.
                let typed = app
                    .providers
                    .iter()
                    .find(|provider| provider.id == wanted)
                    .is_some_and(|provider| provider.needs_key);
                if typed {
                    let label = app
                        .providers
                        .iter()
                        .find(|provider| provider.id == wanted)
                        .map(|provider| provider.label.clone())
                        .unwrap_or_else(|| wanted.clone());
                    app.entering_key = Some((wanted, label.clone()));
                    app.thread_mut()
                        .push(Role::System, format!("Paste the new API key for {label}"));
                    app.thread_mut().clear_input();
                    return true;
                }
                let conn = ctx.conn.clone();
                let tx = ctx.events.clone();
                let session = ctx.session(0);
                tokio::spawn(async move {
                    match conn.login(&wanted, None, Some(session), true).await {
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
                app.thread_mut().clear_input();
                return true;
            }
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.providers().await {
                    Ok(list) => {
                        let _ = tx.send(AgentMsg::Providers(list));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/scoped-models" => {
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.models_all().await {
                    Ok(list) => {
                        let _ = tx.send(AgentMsg::Models(list));
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                    }
                }
            });
            app.thread_mut().clear_input();
            return true;
        }
        "/sessions" => {
            // Only what belongs to this directory: its own sessions, anything
            // started below it, and the global ones. An argument searches the
            // conversations themselves.
            let sessions = if rest.is_empty() {
                store::sessions_in(&ctx.cwd(), &store::home())
            } else {
                store::search_sessions(&ctx.cwd(), &store::home(), rest)
            };
            if sessions.is_empty() {
                app.thread_mut()
                    .push(Role::System, "no saved sessions yet");
            } else {
                app.open_session_picker(sessions);
            }
            app.thread_mut().clear_input();
            return true;
        }
        "/init" => {
            let path = ctx.cwd().join("AGENTS.md");
            if path.exists() {
                app.thread_mut()
                    .push(Role::System, format!("{} already exists", path.display()));
            } else {
                match std::fs::write(&path, AGENTS_TEMPLATE) {
                    Ok(()) => app.thread_mut().push(
                        Role::System,
                        format!("Created {}", app::shorten_cwd(&path)),
                    ),
                    Err(err) => app.thread_mut().push(
                        Role::System,
                        format!("could not write {}: {err}", path.display()),
                    ),
                }
            }
        }
        _ => {
            // Not a built-in command: it may be an installed skill, which runs
            // on the server so its instructions land in the conversation.
            let wanted = name.trim_start_matches('/').to_string();
            if !app
                .skills
                .iter()
                .any(|(skill, _)| skill.trim_start_matches('/') == wanted)
            {
                return false;
            }
            let conn = ctx.conn.clone();
            let session = ctx.session(0);
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                if let Err(err) = conn.skill(&session, &wanted).await {
                    let _ = tx.send(AgentMsg::Error(0, format!("{err:#}")));
                }
            });
            // The server writes the `/skill` line into the transcript itself.
            app.thread_mut().clear_input();
            return true;
        }
    }

    true
}

/// Seeds the side thread with everything the main thread knows, once.
/// `/btw` answers a question in the side thread, with the main context but no
/// tools, while the main turn keeps running.
pub(crate) fn start_side_question(app: &mut App, ctx: &Ctx, question: String) {
    {
        let thread = &mut app.threads[1];
        thread.name = if thread.name.is_empty() {
            thread_name(&question)
        } else {
            thread.name.clone()
        };
        thread.started_at = Some(Instant::now());
        thread.phase = Phase::Streaming;
        thread.error = None;
        thread.live.clear();
    }
    let conn = ctx.conn.clone();
    let session = ctx.session(1);
    tokio::spawn(async move {
        if let Err(err) = conn.prompt(&session, &question, Vec::new(), Vec::new()).await {
            eprintln!("exp-1: side question failed: {err:#}");
        }
    });
}

/// Ticks or unticks one model in the `/scoped-models` list.
pub(crate) fn toggle_scoped(app: &mut App, index: usize) {
    let Some(model) = app.all_models.get(index).cloned() else {
        return;
    };
    let key = app::scope_key(&model);
    // An empty set means "everything", so the first tick has to fill it in.
    if app.scoped.is_empty() {
        app.scoped = app.all_models.iter().map(app::scope_key).collect();
    }
    if !app.scoped.remove(&key) {
        app.scoped.insert(key);
    }
    // Ticking every row is the same as no scope at all.
    let every: std::collections::BTreeSet<String> =
        app.all_models.iter().map(app::scope_key).collect();
    if app.scoped == every {
        app.scoped.clear();
    }
    app.open_scoped_picker();
    if let Some(picker) = app.picker.as_mut() {
        picker.selected = index;
    }
}

pub(crate) fn select_from_picker(app: &mut App, ctx: &Ctx) {
    let Some(picker) = app.picker.take() else {
        return;
    };
    let index = picker.selected;
    let thread = app.active;
    match picker.kind {
        app::PickerKind::Model => {
            // The list is what the picker drew: the rows the scope allows,
            // each knowing which provider it belongs to.
            let Some(row) = app.offered_models().get(index).cloned() else {
                return;
            };
            let slug = row.slug.clone();
            // Choosing a model from another provider means switching to it.
            let switch = (!row.provider.is_empty()
                && app
                    .active_provider_id()
                    .is_none_or(|active| active != row.provider))
            .then(|| row.provider.clone());
            // The levels belong to the row that was picked, and only Codex
            // models have any: a model without them takes the effort as it is.
            let levels: Vec<String> = row.efforts.iter().map(|entry| entry.effort.clone()).collect();
            let settled = app::settle_effort(&levels, row.default_effort.as_deref(), &app.effort);
            app.model = slug.clone();
            app.effort = settled.clone();
            let conn = ctx.conn.clone();
            let (model, effort) = (slug, settled);
            tokio::spawn(async move {
                if let Some(provider) = switch {
                    let _ = conn.login(&provider, None, None, false).await;
                }
                let _ = conn.configure(Some(model), Some(effort)).await;
            });
            // Choosing a model is only half of it when the model has levels to
            // choose; an empty picker after picking a model is just noise.
            if !levels.is_empty() {
                app.open_effort_picker("Select Model and Effort");
            }
        }
        app::PickerKind::Effort => {
            let efforts = app.efforts_for(&app.model);
            let Some((level, _)) = efforts.get(index).cloned() else {
                return;
            };
            app.effort = level.clone();
            let conn = ctx.conn.clone();
            let effort = level;
            tokio::spawn(async move {
                let _ = conn.configure(None, Some(effort)).await;
            });
            let (model, effort) = (app.model_label(), app.effort.clone());
            app.threads[thread]
                .push(Role::System, format!("Model changed to {model} {effort}"));
        }
        app::PickerKind::Provider => {
            // Cloned up front so the borrowed list is released before mutation.
            let Some((id, label, logged_in, active, needs_key, device_login)) =
                app.providers.get(index).map(|provider| {
                    (
                        provider.id.clone(),
                        provider.label.clone(),
                        provider.logged_in,
                        provider.active,
                        provider.needs_key,
                        provider.device_login,
                    )
                })
            else {
                return;
            };
            // A signed-in provider that is not the one in use only needs
            // switching, and the server narrates that. Picking the one already
            // in use is how a sign-in is repeated, so that runs the flow — or
            // asks for a key, for the providers that take one.
            let switching = logged_in && !active;
            if !switching && needs_key && !device_login {
                app.entering_key = Some((id, label.clone()));
                app.thread_mut().push(
                    Role::System,
                    if logged_in {
                        format!("Paste the new API key for {label}, then press Enter")
                    } else {
                        format!("Paste the API key for {label}, then press Enter")
                    },
                );
                return;
            }
            // Only force the flow where there is something to sign in with;
            // an ambient provider has nothing to repeat.
            let force = !switching && (needs_key || device_login);
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            let session = ctx.session(0);
            tokio::spawn(async move {
                match conn.login(&id, None, Some(session), force).await {
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
        }
        app::PickerKind::Scoped => {
            toggle_scoped(app, index);
        }
        app::PickerKind::Message => {
            // The picked row is the choice: 0 copies, 1 goes back.
            let choice = index;
            let Some((message_index, text)) = app.message_target.take() else {
                return;
            };
            if choice == 0 {
                let chars = text.chars().count();
                if copy_to_clipboard(&text) {
                    app.copied = Some((format!("copied {chars} chars"), Instant::now()));
                }
            } else {
                revert_to(app, ctx, message_index, text);
            }
        }
        app::PickerKind::Session => {
            let Some(meta) = app.session_list.get(index).cloned() else {
                return;
            };
            {
                let Ok(mut slots) = ctx.sessions.lock() else {
                    return;
                };
                if slots[0] == meta.id {
                    return;
                }
                slots[0] = meta.id.clone();
            }
            let conn = ctx.conn.clone();
            let tx = ctx.events.clone();
            tokio::spawn(async move {
                match conn.stream(&meta.id).await {
                    Ok(events) => {
                        let _ = tx.send(AgentMsg::Attached(0, meta.label()));
                        // The stream only carries what happens next, so the
                        // conversation so far is fetched as a snapshot.
                        match conn.snapshot(&meta.id).await {
                            Ok(snapshot) => {
                                let _ = tx.send(AgentMsg::Loaded(0, snapshot));
                            }
                            Err(err) => {
                                let _ = tx.send(AgentMsg::Error(
                                    0,
                                    format!("could not read the session: {err:#}"),
                                ));
                            }
                        }
                        pump_events(0, events, tx);
                    }
                    Err(err) => {
                        let _ = tx.send(AgentMsg::Error(
                            0,
                            format!("could not attach to the session: {err:#}"),
                        ));
                    }
                }
            });
        }
        app::PickerKind::Theme => {
            if index >= THEMES.len() {
                return;
            }
            app.theme = index;
            let name = app.theme().name.to_string();
            app.threads[thread].push(Role::System, format!("Theme set to {name}"));
        }
    }
}
/// The files named with `@` in a prompt, resolved against the working
/// directory. A name may be quoted (`@"my file.png"`) when it has spaces.
pub(crate) fn mentions(text: &str, cwd: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '@' {
            index += 1;
            continue;
        }
        // Only a word starting with @ counts, so mail@example.com is not one.
        let at_word_start = index == 0 || chars[index - 1].is_whitespace();
        if !at_word_start {
            index += 1;
            continue;
        }
        index += 1;
        let quoted = chars.get(index) == Some(&'"');
        if quoted {
            index += 1;
        }
        let mut name = String::new();
        while index < chars.len() {
            let ch = chars[index];
            if quoted {
                if ch == '"' {
                    index += 1;
                    break;
                }
            } else if ch.is_whitespace() {
                break;
            }
            name.push(ch);
            index += 1;
        }
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let path = std::path::Path::new(name);
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        found.push(full.to_string_lossy().to_string());
    }
    found
}
