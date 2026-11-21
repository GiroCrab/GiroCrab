//! AppModel+LocalApi.swift (#136): the read-only local interface for external
//! analysis. The core's `LocalApiServer` (a Unix socket, `girocrab-core/src/local_api/server.rs`)
//! runs only while the personal vault is unlocked and shown – never for the demo – and only
//! when the user switched it on and created an access token. Settings UI:
//! `qml/ExternalAccessSettings.qml`, state and job queue: `qml/ExternalAccess.qml`.
//!
//! Linux: the socket lives in `$XDG_RUNTIME_DIR/girocrab/` (Flatpak:
//! `$XDG_RUNTIME_DIR/app/<id>/local-api/`, the part of the runtime dir the host sees)
//! instead of `~/Library/Application Support`. The core creates the folder with mode 0700,
//! the socket with 0600, and closes connections of other users. Tokens never reach a log:
//! only the hash is stored (core), and nothing here prints one.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use girocrab_core::local_api::ShareScope;
use girocrab_core::local_api::server::LocalApiServer;
use girocrab_core::{CoreError, Vault};
use serde_json::{Value, json};

/// `RunningLocalApi`: the socket and the vault it serves.
struct Running {
    server: Arc<LocalApiServer>,
    served: Weak<Vault>,
}

/// `localApi` / `localApiStartError` of the Swift model.
struct State {
    running: Option<Running>,
    start_error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    running: None,
    start_error: None,
});

/// One call of this feature at a time (`localApiJob`): a save, a new key and the start
/// that follows them never interleave (Core.callAsync runs on several worker threads).
static JOB: Mutex<()> = Mutex::new(());

/// Bumped by `cancel_check`: a running connection test ends its helper.
static CHECK_GENERATION: AtomicU64 = AtomicU64::new(0);

/// `AppModel.localApiSocketPath`. Err: no usable runtime dir (shown as the start error).
pub fn socket_path() -> Result<PathBuf, String> {
    socket_path_in(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("FLATPAK_ID").ok(),
    )
}

fn socket_path_in(runtime: Option<PathBuf>, flatpak: Option<String>) -> Result<PathBuf, String> {
    let Some(run) = runtime.filter(|p| p.is_absolute()) else {
        return Err(crate::i18n::tr(
            "XDG_RUNTIME_DIR ist nicht gesetzt: Ohne privaten Laufzeitordner startet die Schnittstelle nicht.",
        ));
    };
    Ok(
        match flatpak.filter(|id| !id.is_empty() && !id.contains('/')) {
            Some(id) => run.join("app").join(id).join("local-api/girocrab.sock"),
            None => run.join("girocrab/girocrab.sock"),
        },
    )
}

/// `AppModel.localApiHelperURL`: `girocrab-mcp` next to the app's executable (the .deb
/// installs both into /usr/bin), in a developer build the repository's newer of
/// `target/release` and `target/debug` (looked up each time, so a helper built while the
/// app runs is found). None = not built.
pub fn helper_path() -> Option<PathBuf> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("girocrab-mcp")))
        .filter(|p| is_executable(p));
    if bundled.is_some() {
        return bundled;
    }
    #[cfg(debug_assertions)]
    {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let modified = |p: &PathBuf| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        };
        ["release", "debug"]
            .iter()
            .map(|t| root.join("target").join(t).join("girocrab-mcp"))
            .filter(|p| is_executable(p))
            .max_by_key(modified)
            .and_then(|p| p.canonicalize().ok())
    }
    #[cfg(not(debug_assertions))]
    None
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `AppModel.localApiClientConfig`: the entry for an MCP client's configuration (the common
/// `mcpServers` form) with the helper's path and `token` (or a placeholder). Linux: the
/// helper has no default socket path outside macOS, so the entry names it.
pub fn client_config(helper: &str, socket: &str, token: Option<&str>) -> String {
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    format!(
        "{{\n  \"mcpServers\": {{\n    \"girocrab\": {{\n      \"command\": {},\n      \"env\": {{\n        \"GIROCRAB_API_SOCKET\": {},\n        \"GIROCRAB_API_TOKEN\": {}\n      }}\n    }}\n  }}\n}}",
        q(helper),
        q(socket),
        q(token.unwrap_or("gcapi_…"))
    )
}

fn describe(e: &CoreError) -> String {
    match e {
        // The core words its start errors for the user (localized).
        CoreError::InvalidInput { message } => message.clone(),
        e => e.to_string(),
    }
}

/// `stopLocalApiServer`: closes the socket and removes its file (also on quit, lock and the
/// demo), and clears a start error. Returns at once: the core's `stop()` does not wait.
pub fn stop() {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = s.running.take() {
        r.server.stop();
    }
    s.start_error = None;
}

/// `applyLocalApiScope` for `target`: read the stored scope, then start, keep or stop the
/// socket. Returns the stored scope (None when it could not be read: start error set).
fn reconcile(target: &Arc<Vault>) -> Option<ShareScope> {
    let scope = match target.local_api_scope() {
        Ok(s) => s,
        Err(e) => {
            stop();
            STATE.lock().unwrap_or_else(|e| e.into_inner()).start_error = Some(describe(&e));
            return None;
        }
    };
    if !scope.enabled || scope.token_sha256.is_empty() {
        stop();
        return Some(scope);
    }
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if s.running
        .as_ref()
        .and_then(|r| r.served.upgrade())
        .is_some_and(|v| Arc::ptr_eq(&v, target))
    {
        return Some(scope);
    }
    if let Some(r) = s.running.take() {
        r.server.stop();
    }
    s.start_error = None;
    let started = socket_path().and_then(|p| {
        LocalApiServer::start(target.clone(), p.display().to_string()).map_err(|e| describe(&e))
    });
    match started {
        Ok(server) => {
            s.running = Some(Running {
                server,
                served: Arc::downgrade(target),
            })
        }
        // Kept for the status line („Nicht gestartet: …“).
        Err(e) => s.start_error = Some(e),
    }
    Some(scope)
}

/// What the settings pane shows: the stored scope and the socket's state.
fn state(scope: Option<ShareScope>) -> Value {
    let s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "scope": scope,
        "running": s.running.is_some(),
        "start_error": s.start_error,
        "socket_path": socket_path().map(|p| p.display().to_string()).unwrap_or_default(),
        "helper_path": helper_path().map(|p| p.display().to_string()),
        "dev_build": cfg!(debug_assertions),
    })
}

/// One job of the queue on `target` (the own vault, unlocked and shown): `work`, then the
/// stored scope and the socket are matched to it. `{ value, error, state }`: a failed
/// write still reports what is stored, so the pane goes back to it.
pub fn job<T: serde::Serialize>(
    target: Arc<Vault>,
    work: impl FnOnce(&Vault) -> Result<T, CoreError>,
) -> Value {
    let _job = JOB.lock().unwrap_or_else(|e| e.into_inner());
    let (value, error) = match work(&target) {
        Ok(v) => (serde_json::to_value(v).unwrap_or(Value::Null), None),
        Err(e) => (Value::Null, Some(e.to_string())),
    };
    let scope = reconcile(&target);
    json!({ "value": value, "error": error, "state": state(scope) })
}

/// `checkLocalApiConnection` result names (`LocalApiCheck`), mapped from
/// `girocrab-mcp --check --json` (`Problem::name`); `failed` carries the helper's message.
fn check_result(stdout: &str, stderr: &str) -> Value {
    let answer: Option<Value> = serde_json::from_str(stdout.trim()).ok();
    let first_line = |t: &str| t.lines().next().map(str::to_string);
    let message = answer
        .as_ref()
        .and_then(|a| a["message"].as_str().map(str::to_string))
        .or_else(|| first_line(stderr))
        .or_else(|| first_line(stdout))
        .unwrap_or_default();
    let result = match answer.as_ref().and_then(|a| a["problem"].as_str()) {
        Some("ok") => "works",
        Some("unreachable") => "notReachable",
        Some("no_key") => "reachableWithoutKey",
        Some("key_rejected") => "keyRejected",
        Some("disabled") => "turnedOff",
        Some("locked") => "locked",
        Some("busy") => "busy",
        _ => "failed",
    };
    json!({ "result": result, "message": message })
}

/// Longer than the helper's own worst case (30 s for one answer, then a few short
/// retries), so its own message arrives rather than a kill.
const HELPER_CHECK_LIMIT: Duration = Duration::from_secs(45);

/// Runs `helper --check --json` the way an MCP client starts it, on this app's socket path
/// and with the key just created if it is still shown (otherwise without one: then only
/// reachability is known). `{ result: "noAnswer" }` when it had to be stopped (time limit or
/// `cancel_check`). The key goes in through the environment, never on the command line.
pub fn check(token: Option<String>) -> Value {
    let generation = CHECK_GENERATION.load(Ordering::SeqCst);
    let no_answer = || json!({ "result": "noAnswer", "message": "" });
    let Some(helper) = helper_path() else {
        return json!({ "result": "failed", "message": "girocrab-mcp" });
    };
    let mut cmd = Command::new(&helper);
    cmd.args(["--check", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match socket_path() {
        Ok(p) => cmd.env("GIROCRAB_API_SOCKET", p),
        Err(_) => cmd.env_remove("GIROCRAB_API_SOCKET"),
    };
    match token.filter(|t| !t.is_empty()) {
        Some(t) => cmd.env("GIROCRAB_API_TOKEN", t),
        None => cmd.env_remove("GIROCRAB_API_TOKEN"),
    };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({ "result": "failed", "message": e.to_string() }),
    };
    // Read both pipes on their own threads so a chatty helper never blocks on a full pipe.
    let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut out);
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    );
    let deadline = Instant::now() + HELPER_CHECK_LIMIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (stdout, stderr) = (
                    out.join().unwrap_or_default(),
                    err.join().unwrap_or_default(),
                );
                // ended by a signal: like Process.terminationReason != .exit
                if status.code().is_none() {
                    return no_answer();
                }
                return check_result(&stdout, &stderr);
            }
            Ok(None) => {}
            Err(e) => return json!({ "result": "failed", "message": e.to_string() }),
        }
        if Instant::now() >= deadline || CHECK_GENERATION.load(Ordering::SeqCst) != generation {
            let _ = child.kill();
            let _ = child.wait();
            return no_answer();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A new or revoked key, or the pane closed: a running test is stale, its helper ends.
pub fn cancel_check() {
    CHECK_GENERATION.fetch_add(1, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_lives_in_the_runtime_dir() {
        let run = Some(PathBuf::from("/run/user/1000"));
        assert_eq!(
            socket_path_in(run.clone(), None).unwrap(),
            PathBuf::from("/run/user/1000/girocrab/girocrab.sock")
        );
        assert_eq!(
            socket_path_in(run.clone(), Some("de.girocrab.GiroCrab".into())).unwrap(),
            PathBuf::from("/run/user/1000/app/de.girocrab.GiroCrab/local-api/girocrab.sock")
        );
        assert!(socket_path_in(None, None).is_err());
        assert!(socket_path_in(Some(PathBuf::from("run")), None).is_err());
        assert_eq!(
            socket_path_in(run, Some("../x".into())).unwrap(),
            PathBuf::from("/run/user/1000/girocrab/girocrab.sock"),
            "an odd FLATPAK_ID never leaves the runtime dir"
        );
    }

    #[test]
    fn client_config_is_json_with_socket_and_placeholder() {
        let c = client_config(
            "/usr/bin/girocrab-mcp",
            "/run/user/1000/girocrab/girocrab.sock",
            None,
        );
        let v: Value = serde_json::from_str(&c).unwrap();
        let s = &v["mcpServers"]["girocrab"];
        assert_eq!(s["command"], "/usr/bin/girocrab-mcp");
        assert_eq!(
            s["env"]["GIROCRAB_API_SOCKET"],
            "/run/user/1000/girocrab/girocrab.sock"
        );
        assert_eq!(s["env"]["GIROCRAB_API_TOKEN"], "gcapi_…");
        let quoted = client_config("/a \"b\"/girocrab-mcp", "/s", Some("gcapi_1"));
        let v: Value = serde_json::from_str(&quoted).unwrap();
        assert_eq!(
            v["mcpServers"]["girocrab"]["command"],
            "/a \"b\"/girocrab-mcp"
        );
        assert_eq!(
            v["mcpServers"]["girocrab"]["env"]["GIROCRAB_API_TOKEN"],
            "gcapi_1"
        );
    }

    #[test]
    fn check_answers_map_like_the_mac() {
        let r = |p: &str| {
            check_result(&format!("{{\"problem\":\"{p}\",\"message\":\"m\"}}"), "")["result"]
                .clone()
        };
        assert_eq!(r("ok"), "works");
        assert_eq!(r("unreachable"), "notReachable");
        assert_eq!(r("no_key"), "reachableWithoutKey");
        assert_eq!(r("key_rejected"), "keyRejected");
        assert_eq!(r("disabled"), "turnedOff");
        assert_eq!(r("locked"), "locked");
        assert_eq!(r("busy"), "busy");
        let other = check_result("", "error: unknown flag\nmore");
        assert_eq!(other["result"], "failed");
        assert_eq!(other["message"], "error: unknown flag");
    }
}
