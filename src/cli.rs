//! the command line, the help text and the service subcommands.

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    LeaveAlternateScreen, disable_raw_mode,
};
use std::path::PathBuf;

use super::*;

pub(crate) const AGENTS_TEMPLATE: &str = "\
# AGENTS.md

Instructions for exp-1 agent in this repository.

## Commands
- Build:
- Test:
- Lint / format:
- Run:

## Conventions
- Describe the code layout a newcomer should know first.
- Note any tooling that must run before a change is testable.
- Record conventions that are not obvious from the code itself.
";

#[derive(Clone)]
pub(crate) struct Args {
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) cwd: PathBuf,
}

pub(crate) fn parse_args() -> Result<Args> {
    let mut model = std::env::var("EXP1_MODEL").ok();
    let mut effort: Option<String> = None;
    let mut cwd = std::env::current_dir().context("resolve current directory")?;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--model" | "-m" => {
                model = Some(argv.next().context("--model needs a value")?);
            }
            "--effort" => {
                effort = Some(argv.next().context("--effort needs a value")?);
            }
            "--cwd" => {
                cwd = PathBuf::from(argv.next().context("--cwd needs a path")?);
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument '{other}' (try --help)"),
        }
    }

    Ok(Args { model, effort, cwd })
}

fn print_help() {
    println!(
        "exp-1 agent {}\n\n\
         USAGE:\n    exp1 [OPTIONS]\n    exp1 install <skill>  install a skill, run it with /<name>\n\
         \x20   exp1 service <start|stop|restart|status|password>\n\n\
         OPTIONS:\n\
         \x20   -m, --model <NAME>   Model to use (default: the catalogue pick, env: EXP1_MODEL)\n\
         \x20       --effort <LEVEL>  Reasoning effort: low | medium | high (default: medium)\n\
         \x20       --cwd <PATH>      Working directory for tools\n\
         \x20   -h, --help           Show this help\n\n\
         TOOLS:\n\
         \x20 read   read a file; limit limits lines, pos sets the first line\n\
         \x20 write  create or overwrite a file\n\
         \x20 edit   replace an exact snippet, failing when it is not unique\n\
         \x20 delete remove a file (directories are refused; no undo)\n\
         \x20 shell  run a command; background true keeps the agent working\n\
         \x20 ast    show the project tree, cached in .cache/ast\n\
         \x20 count  count lines in a file or a tree, cached in .cache/count\n\
         \x20 web    search the web (Exa first, DuckDuckGo as the fallback)\n\
         \x20 fetch  read one web page as text; limit lines, pos starts at a line\n\
         \x20 task   spawn a subagent to do focused work in its own context\n\
         \x20 ask    ask the user a question and wait for the answer\n\n\
         COMMANDS:\n\
         \x20 /login          pick a provider; one that is signed in just switches\n\
         \x20 /login <id>     sign in again (device page, or a fresh key)\n\
         \x20 /scoped-models  choose which models /model offers\n\
         \x20 /sessions       reopen an earlier conversation\n\
         \x20 /retitle       name the session again from the conversation\n\
         \x20 /effort        pick the reasoning effort for the current model\n\
         \x20 /mcp           list the MCP servers and their tools\n\
         \x20 /compact       summarise the conversation now\n\
         \x20 /undo /diff     restore or inspect what the last turn changed\n\
         \x20 /fork          continue as a new session\n\
         \x20 /retry <1-10>   configure request attempts\n\
         \x20 /model /themes /status /init /clear /btw /help\n\n\
         KEYS:\n\
         \x20 ctrl + /   switch between the main thread and the side thread\n\
         \x20 shift+tab  switch between build and plan mode\n\
         \x20 esc esc    interrupt the running turn\n\n\
         AUTH:\n\
         \x20 Run /login in the app. openai (auth), github copilot and meta (auth) sign in\n\
         \x20 through the vendor's device-code page, which opens in your browser; the rest\n\
         \x20 take an API key. Picking a provider that is signed in switches to it, and\n\
         \x20 picking the one already in use signs in again. google vertex uses gcloud's\n\
         \x20 application-default credentials, and the Codex CLI's own auth.json is read\n\
         \x20 as a fallback and never rewritten.",
        env!("CARGO_PKG_VERSION")
    );
}
pub(crate) async fn service_command(rest: &[String]) -> Result<()> {
    match rest.first().map(String::as_str) {
        Some("start") => {
            let url = daemon::ensure_running().await?;
            println!("exp-1 server running at {url}");
        }
        Some("stop") => match daemon::read_registration() {
            Some(registration) => {
                daemon::stop(&registration);
                println!("stopped server {}", registration.pid);
            }
            None => println!("no server is registered"),
        },
        Some("status") => match daemon::read_registration() {
            Some(registration) => {
                let alive = daemon::is_healthy(&registration).await;
                let current = daemon::build_id();
                let build = registration.build.as_deref().unwrap_or("unknown");
                let fits = if registration.build == current {
                    "matches this binary"
                } else {
                    "was replaced since; a launch would restart it"
                };
                println!(
                    "pid {}\nurl {}\nversion {}\nbuild {} ({fits})\nhealthy {}",
                    registration.pid,
                    registration.url,
                    registration.version,
                    build,
                    if alive { "yes" } else { "no" }
                );
            }
            None => println!("no server is registered"),
        },
        Some("restart") => {
            if let Some(registration) = daemon::read_registration() {
                daemon::stop(&registration);
            }
            let url = daemon::ensure_running().await?;
            println!("exp-1 server running at {url}");
        }
        Some("password") => match rest.get(1) {
            Some(value) => {
                daemon::set_password(value)?;
                println!("password updated");
            }
            None => println!("{}", daemon::password()),
        },
        other => {
            anyhow::bail!("unknown service command {other:?} (start|stop|status|restart|password)")
        }
    }
    Ok(())
}
/// Writes a panic where it can be found, and leaves the terminal usable: a
/// panic in raw mode otherwise looks exactly like a frozen screen, with every
/// keystroke going nowhere.
pub(crate) fn install_panic_hook() {
    let state = daemon::state_dir();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            crossterm::cursor::Show,
            LeaveAlternateScreen
        );
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let _ = std::fs::create_dir_all(&state);
        let path = state.join("crash.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = writeln!(
                file,
                "--- {} ---\n{message}\n  at {location}\n",
                store::now_iso()
            );
        }
        // The screen is back to the shell by now, so the reason is readable
        // instead of the window simply looking frozen.
        eprintln!(
            "\nexp-1 agent stopped: {message}\n  at {location}\n  written to {}\n",
            path.display()
        );
    }));
}

/// Anything that takes noticeably long lands in `slow.log`, so a freeze that
/// cannot be reproduced still leaves a trace of what was slow.
pub(crate) fn note_slow(what: &str, took: std::time::Duration) {
    if took < std::time::Duration::from_millis(750) {
        return;
    }
    let state = daemon::state_dir();
    let _ = std::fs::create_dir_all(&state);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("slow.log"))
    {
        use std::io::Write;
        let _ = writeln!(file, "{} {what}: {took:?}", store::now_iso());
    }
}
