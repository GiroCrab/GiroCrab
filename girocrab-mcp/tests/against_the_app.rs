//! End to end: the adapter against the real socket server of the core, the way
//! the app runs it.

mod common;

use common::ShortDir;
use girocrab_core::local_api::server::LocalApiServer;
use girocrab_core::local_api::ShareScope;
use girocrab_core::Vault;
use girocrab_mcp::{Adapter, Config};
use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

#[test]
fn tools_read_what_the_user_shares_and_stop_with_the_app() {
    let dir = ShortDir::new();
    let vault = Vault::open(dir.0.join("v.db").to_string_lossy().into(), vec![7u8; 32]).unwrap();
    vault.seed_demo_data().unwrap();
    let first = vault.list_accounts().unwrap()[0].id;
    vault
        .set_local_api_scope(ShareScope {
            enabled: true,
            account_ids: vec![first],
            ..Default::default()
        })
        .unwrap();
    let token = vault.create_local_api_token().unwrap();
    let socket = dir.0.join("api").join("s.sock");
    let server = LocalApiServer::start(vault.clone(), socket.to_string_lossy().into()).unwrap();

    let mut mcp = Adapter::new(Config {
        socket: socket.clone(),
        token: Ok(token),
    });
    let call = |mcp: &mut Adapter, tool: &str, args: Value| -> Value {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": { "name": tool, "arguments": args } });
        mcp.handle(&msg).unwrap()["result"].clone()
    };

    let info = call(&mut mcp, "get_info", json!({}));
    assert_eq!(info["isError"], false);
    assert_eq!(info["structuredContent"]["account_count"], 1);
    let accounts = call(&mut mcp, "list_accounts", json!({}));
    assert_eq!(accounts["structuredContent"]["items"][0]["id"], first);
    let page = call(&mut mcp, "list_transactions", json!({ "limit": 3 }));
    let bookings = page["structuredContent"]["transactions"]
        .as_array()
        .unwrap();
    assert_eq!(bookings.len(), 3);
    assert!(
        bookings.iter().all(|b| b["account_id"] == first),
        "only the shared account"
    );
    let summary = call(
        &mut mcp,
        "get_summary",
        json!({ "date_from": "2025-01-01", "date_to": "2026-12-31" }),
    );
    assert_eq!(summary["isError"], false);
    let wrong = call(&mut mcp, "list_transactions", json!({ "nonsense": 1 }));
    assert_eq!(wrong["isError"], true, "the app checks the arguments");

    server.stop();
    let gone = call(&mut mcp, "get_info", json!({}));
    assert_eq!(gone["isError"], true);
    assert!(gone["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not reachable"));
}

/// A demo vault sharing its first account, served on a socket in `dir`.
fn serve(dir: &ShortDir) -> (Arc<Vault>, Arc<LocalApiServer>, String, std::path::PathBuf) {
    let vault = Vault::open(dir.0.join("v.db").to_string_lossy().into(), vec![7u8; 32]).unwrap();
    vault.seed_demo_data().unwrap();
    let first = vault.list_accounts().unwrap()[0].id;
    vault
        .set_local_api_scope(ShareScope {
            enabled: true,
            account_ids: vec![first],
            ..Default::default()
        })
        .unwrap();
    let token = vault.create_local_api_token().unwrap();
    let socket = dir.0.join("api").join("s.sock");
    let server = LocalApiServer::start(vault.clone(), socket.to_string_lossy().into()).unwrap();
    (vault, server, token, socket)
}

#[test]
fn the_apps_connection_limit_is_waited_out() {
    let dir = ShortDir::new();
    let (_vault, server, token, socket) = serve(&dir);
    // Four other programs hold every connection the app allows.
    let held: Vec<UnixStream> = (0..4)
        .map(|_| UnixStream::connect(&socket).unwrap())
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        drop(held);
    });
    let mut mcp = Adapter::new(Config {
        socket: socket.clone(),
        token: Ok(token.clone()),
    });
    let info = mcp.request("info", json!({})).unwrap();
    assert_eq!(
        info["account_count"], 1,
        "answered once a connection was free"
    );
    release.join().unwrap();

    // Held for good: the user is told GiroCrab is busy.
    let _held: Vec<UnixStream> = (0..4)
        .map(|_| UnixStream::connect(&socket).unwrap())
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let mut other = Adapter::new(Config {
        socket,
        token: Ok(token),
    });
    let err = other.request("info", json!({})).unwrap_err();
    assert!(err.0.contains("busy"), "{}", err.0);
    server.stop();
}
