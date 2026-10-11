//! Background server lifecycle, modelled on opencode's daemon service.
//!
//! A single detached `exp1 serve` process serves every UI on the machine.
//! Clients find it through a registration file in the state directory, and
//! authenticate with a password that persists across restarts so a new client
//! can reconnect without being told anything.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long to wait for a freshly spawned server to answer `/health`.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for a server to exit after SIGTERM before forcing it.
const STOP_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registration {
    /// Random per-server id, so a client can tell two servers apart.
    pub id: String,
    pub version: String,
    /// Base URL, for example `http://127.0.0.1:41234`.
    pub url: String,
    pub pid: u32,
    /// Which build of the binary that server runs. The wire changes between
    /// builds while the crate version stays put, so a client rebuilt since the
    /// server started must replace it rather than talk a protocol it no longer
    /// speaks. `None` for registrations written before this was recorded, which
    /// are treated as stale for the same reason.
    #[serde(default)]
    pub build: Option<String>,
}

/// Identifies the executable: its size and modification time both change on a
/// rebuild, which is precisely when the protocol may have changed too.
pub fn build_id() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let meta = std::fs::metadata(&exe).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{:x}-{modified:x}", meta.len()))
}

pub fn state_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("XDG_STATE_HOME").filter(|base| !base.is_empty()) {
        return PathBuf::from(base).join("exp-1");
    }
    // Windows has no `~/.local/state`; LOCALAPPDATA is where a daemon's own
    // files belong there.
    #[cfg(windows)]
    if let Some(base) = std::env::var_os("LOCALAPPDATA").filter(|base| !base.is_empty()) {
        return PathBuf::from(base).join("exp-1");
    }
    match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        Some(home) => PathBuf::from(home).join(".local/state/exp-1"),
        None => PathBuf::from("/tmp/exp-1-state"),
    }
}

fn registration_path() -> PathBuf {
    state_dir().join("server.json")
}

fn password_path() -> PathBuf {
    state_dir().join("password")
}

/// Reads the shared password, creating one on first use.
pub fn password() -> String {
    if let Ok(existing) = std::fs::read_to_string(password_path())
        && !existing.trim().is_empty()
    {
        return existing.trim().to_string();
    }
    let fresh = random_token();
    let _ = std::fs::create_dir_all(state_dir());
    let _ = std::fs::write(password_path(), &fresh);
    restrict(&password_path());
    fresh
}

/// Overwrites the password, used by `exp1 service password`.
pub fn set_password(value: &str) -> Result<()> {
    std::fs::create_dir_all(state_dir()).context("create state directory")?;
    std::fs::write(password_path(), value).context("write password")?;
    restrict(&password_path());
    Ok(())
}

fn restrict(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 32 random bytes, base64url — long enough that guessing is pointless.
fn random_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    // getrandom is already in the tree through uuid; read /dev/urandom instead
    // of adding a direct dependency for one call.
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = file.read_exact(&mut bytes);
    } else {
        let seed = uuid::Uuid::new_v4();
        bytes[..16].copy_from_slice(seed.as_bytes());
        let second = uuid::Uuid::new_v4();
        bytes[16..].copy_from_slice(second.as_bytes());
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn random_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn read_registration() -> Option<Registration> {
    let raw = std::fs::read_to_string(registration_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_registration(registration: &Registration) -> Result<()> {
    std::fs::create_dir_all(state_dir()).context("create state directory")?;
    let json = serde_json::to_vec_pretty(registration)?;
    let path = registration_path();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, json).context("write registration")?;
    std::fs::rename(&tmp, &path).context("replace registration")?;
    Ok(())
}

pub fn clear_registration() {
    let _ = std::fs::remove_file(registration_path());
}

/// `GET /health`, with a short timeout so a dead server is spotted quickly.
pub async fn is_healthy(registration: &Registration) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    match client
        .get(format!("{}/health", registration.url))
        .header("authorization", format!("Bearer {}", password()))
        .send()
        .await
    {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 performs the permission and existence checks without sending.
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // Windows has no signal 0; `tasklist` is present on every install.
        let filter = format!("PID eq {pid}");
        match std::process::Command::new("tasklist")
            .args(["/FI", &filter, "/NH"])
            .output()
        {
            Ok(output) => String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

fn signal(pid: u32, sig: i32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, sig);
    }
    #[cfg(windows)]
    {
        // `taskkill` without /F asks the process to close; /F is the SIGKILL
        // step. /T takes the children with it, which a server has.
        let mut command = std::process::Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T"]);
        if sig == 9 {
            command.arg("/F");
        }
        let _ = command.output();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, sig);
    }
}

/// SIGTERM, wait, then SIGKILL — the same escalation opencode uses.
pub fn stop(registration: &Registration) {
    signal(registration.pid, 15);
    let deadline = std::time::Instant::now() + STOP_GRACE;
    while std::time::Instant::now() < deadline {
        if !process_alive(registration.pid) {
            clear_registration();
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    signal(registration.pid, 9);
    std::thread::sleep(Duration::from_millis(100));
    clear_registration();
}

/// Returns the URL of a healthy server, reusing or spawning one as needed.
pub async fn ensure_running() -> Result<String> {
    if let Some(existing) = read_registration() {
        let build = build_id();
        // Reuse only a server that is the same version *and* the same build;
        // anything else may speak a different wire than this client.
        let same_build = build.is_some() && existing.build == build;
        if existing.version == env!("CARGO_PKG_VERSION")
            && same_build
            && is_healthy(&existing).await
        {
            return Ok(existing.url);
        }
        // Stale, rebuilt, foreign-version, or dead: clear it out before starting a new one.
        if process_alive(existing.pid) {
            stop(&existing);
        } else {
            clear_registration();
        }
    }
    spawn_detached()?;

    let deadline = std::time::Instant::now() + START_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(registration) = read_registration()
            && is_healthy(&registration).await
        {
            return Ok(registration.url);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("the background server did not come up within {START_TIMEOUT:?}")
}

/// Launches `exp1 serve` as a detached child that outlives this process.
fn spawn_detached() -> Result<()> {
    let exe = std::env::current_exe().context("locate the exp1 binary")?;
    let mut command = Command::new(exe);
    command
        .arg("serve")
        .arg("--register")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A new session detaches it from this process group, so it survives.
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: the server outlives the
        // console that started it instead of dying with it.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    command.spawn().context("spawn the background server")?;
    Ok(())
}

/// Runs `serve` in the foreground, registering itself so clients can find it.
///
/// The port is bound first so the registration carries the real one.
pub async fn run_foreground(register: bool, port: Option<u16>) -> Result<()> {
    let listener = crate::server::bind(port).await?;
    let bound = listener.local_addr().context("read the bound port")?.port();
    if register {
        let registration = Registration {
            id: random_id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            url: format!("http://127.0.0.1:{bound}"),
            pid: std::process::id(),
            build: build_id(),
        };
        write_registration(&registration)?;
    }
    crate::server::serve(listener, bound).await
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_id_is_stable_within_one_binary() {
        let first = build_id().expect("the test binary has a path");
        assert_eq!(Some(first), build_id());
    }

    /// A registration written before the build was recorded has no `build`
    /// field; it must still parse, and reads as unknown so the client replaces
    /// that server instead of talking a protocol it may not speak.
    #[test]
    fn an_old_registration_reads_as_unknown_build() {
        let raw = r#"{"id":"abc","version":"0.1.0","url":"http://127.0.0.1:1","pid":1}"#;
        let registration: Registration = serde_json::from_str(raw).expect("parse");
        assert_eq!(registration.build, None);
        assert_ne!(registration.build, build_id());
    }
}
