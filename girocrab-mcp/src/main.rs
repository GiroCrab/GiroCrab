//! `girocrab-mcp`: started by an MCP client, talks JSON-RPC over stdin/stdout.
//! Logs go to stderr only (stdout carries the protocol).

use girocrab_mcp::{rpc_error, Adapter, Config};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::PathBuf;

const USAGE: &str = "\
girocrab-mcp: MCP server (stdio) for GiroCrab's read-only local interface

Usage: girocrab-mcp [--socket <path>] [--token-file <path>]

The access key comes from GiroCrab (Settings › Analysis). Pass it in the
environment as GIROCRAB_API_TOKEN, or as a file with --token-file; never as an
argument (other programs can read the command line).

  --socket <path>      socket of the app (default: GIROCRAB_API_SOCKET, or
                       ~/Library/Application Support/GiroCrab/local-api/girocrab.sock)
  --token-file <path>  file holding the access key (should be readable only by you)
  --version            print the version
";

fn main() {
    let config = match config(std::env::args().skip(1).collect()) {
        Ok(c) => c,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if config.token.is_none() {
        // Still serve: the client then shows the tool error that explains the setup.
        eprintln!("girocrab-mcp: no access key (GIROCRAB_API_TOKEN or --token-file)");
    }
    let mut adapter = Adapter::new(config);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => adapter.handle(&msg),
            Err(e) => Some(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(answer) = answer {
            if writeln!(stdout, "{answer}")
                .and_then(|_| stdout.flush())
                .is_err()
            {
                break; // the client went away
            }
        }
    }
}

fn config(args: Vec<String>) -> Result<Config, String> {
    let mut socket = std::env::var_os("GIROCRAB_API_SOCKET").map(PathBuf::from);
    let mut token = std::env::var("GIROCRAB_API_TOKEN").ok();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = Some(args.next().ok_or("--socket needs a path")?.into()),
            "--token-file" => {
                let path = args.next().ok_or("--token-file needs a path")?;
                token = Some(read_token_file(&path)?);
            }
            "--version" => {
                println!("girocrab-mcp {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}\n\n{USAGE}")),
        }
    }
    let socket = match socket {
        Some(s) => s,
        None => default_socket().ok_or("no socket path: pass --socket or GIROCRAB_API_SOCKET")?,
    };
    let token = token
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(Config { socket, token })
}

fn read_token_file(path: &str) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| format!("--token-file {path}: {e}"))?;
    if meta.permissions().mode() & 0o077 != 0 {
        eprintln!("girocrab-mcp: {path} can be read by other users; chmod 600 it");
    }
    std::fs::read_to_string(path).map_err(|e| format!("--token-file {path}: {e}"))
}

/// Where the macOS app puts its socket.
fn default_socket() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/GiroCrab/local-api/girocrab.sock"))
}
