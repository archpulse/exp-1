//! the desktop around the app: clipboard, bell, notifications.


use super::*;

/// Rings the terminal bell and, when the desktop has one, shows a notification.
/// Both are best effort: a headless machine simply gets the bell.
pub(crate) fn ring_attention() {
    say("exp-1 agent", "the turn has finished");
}

/// The bell plus a desktop notification with words of our own: what a scheduled
/// job did, or that the turn is over.
pub(crate) fn say(title: &str, body: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x07");
    let _ = out.flush();

    // Every branch compiles everywhere; `cfg!` only picks which one runs.
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("osascript");
        command.args([
            "-e",
            &format!(
                "display notification {} with title {}",
                applescript_text(body),
                applescript_text(title)
            ),
        ]);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("powershell");
        command.args([
            "-NoProfile",
            "-Command",
            &format!(
                "Add-Type -AssemblyName System.Windows.Forms; \
                 $n = New-Object System.Windows.Forms.NotifyIcon; \
                 $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true; \
                 $n.ShowBalloonTip(5000, '{}', '{}', 'Info')",
                title.replace('\'', "''"),
                body.replace('\'', "''"),
            ),
        ]);
        command
    } else {
        let mut command = std::process::Command::new("notify-send");
        command.args([title, body]);
        command
    };
    let _ = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// A string as an AppleScript literal: quotes and backslashes escaped.
fn applescript_text(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    )
}
/// Takes an image off the clipboard and holds it for the next prompt. Wayland
/// first, then X11; a missing tool is simply "nothing to paste".
pub(crate) fn attach_clipboard_image(app: &mut App) {
    use base64::Engine;
    match clipboard_image() {
        Some((media_type, bytes)) => {
            let size = if bytes.len() >= 1024 {
                format!("{} KB", bytes.len() / 1024)
            } else {
                format!("{} bytes", bytes.len())
            };
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            app.pending_images.push((media_type.clone(), data));
            let queued = app.pending_images.len();
            app.thread_mut().push(
                Role::System,
                format!("image attached: {media_type}, {size} — {queued} queued for the next prompt"),
            );
        }
        None => app.thread_mut().push(
            Role::System,
            "no image in the clipboard (nothing copied, or wl-paste/xclip is missing)",
        ),
    }
}

/// The raw bytes of the clipboard's first image, with its media type.
fn clipboard_image() -> Option<(String, Vec<u8>)> {
    // Each platform has its own way to the clipboard, and the body for one is
    // the whole function: only one of these blocks ever compiles.
    #[cfg(target_os = "macos")]
    {
        // macOS has no built-in CLI for images on the clipboard; `pngpaste` is
        // the tool people install for exactly this.
        match std::process::Command::new("pngpaste").arg("-").output() {
            Ok(out) if out.status.success() && !out.stdout.is_empty() => {
                Some(("image/png".to_string(), out.stdout))
            }
            _ => None,
        }
    }
    #[cfg(target_os = "windows")]
    {
        // PowerShell writes the clipboard image to a file, which is then read
        // back and deleted.
        let path = std::env::temp_dir().join(format!("exp1-clipboard-{}.png", std::process::id()));
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $img = [System.Windows.Forms.Clipboard]::GetImage(); \
             if ($img) {{ $img.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png) }}",
            path.display()
        );
        let saved = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        let bytes = if saved {
            std::fs::read(&path).ok().filter(|bytes| !bytes.is_empty())
        } else {
            None
        };
        let _ = std::fs::remove_file(&path);
        bytes.map(|bytes| ("image/png".to_string(), bytes))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Wayland: ask which types are offered, then take the first image one.
        if let Ok(out) = std::process::Command::new("wl-paste")
            .arg("--list-types")
            .output()
            && out.status.success()
        {
            let types = String::from_utf8_lossy(&out.stdout);
            if let Some(media) = types
                .lines()
                .map(str::trim)
                .find(|line| line.starts_with("image/"))
                && let Ok(bytes) = std::process::Command::new("wl-paste")
                    .args(["--no-newline", "--type", media])
                    .output()
                && bytes.status.success()
                && !bytes.stdout.is_empty()
            {
                return Some((media.to_string(), bytes.stdout));
            }
        }
        // X11, for the sessions that are not Wayland.
        for media in ["image/png", "image/jpeg"] {
            if let Ok(out) = std::process::Command::new("xclip")
                .args(["-selection", "clipboard", "-t", media, "-o"])
                .output()
                && out.status.success()
                && !out.stdout.is_empty()
            {
                return Some((media.to_string(), out.stdout));
            }
        }
        None
    }
}

/// Puts text on the clipboard. OSC 52 hands it to the terminal itself, which
/// is the only route that also works over ssh; the desktop tools are tried
/// after that so a local session gets it even where OSC 52 is ignored. A
/// missing tool is not a failure: the other routes still count.
pub(crate) fn copy_to_clipboard(text: &str) -> bool {
    use base64::Engine;
    use std::io::Write;

    let mut delivered = false;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = std::io::stdout();
    if write!(out, "\x1b]52;c;{encoded}\x07").is_ok() && out.flush().is_ok() {
        delivered = true;
    }

    let candidates: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["-ib"]),
    ];
    for (program, args) in candidates {
        let Ok(mut child) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        drop(child.stdin.take());
        // Reaped off the event loop, so no zombie lingers in a long session.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        delivered = true;
        break;
    }
    delivered
}
