use super::*;
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};

/// A short directory: `sun_path` holds only 104 bytes on macOS.
struct ShortDir(PathBuf);

impl ShortDir {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = PathBuf::from(format!("/tmp/gcm-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("s.sock")
    }
}

impl Drop for ShortDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Stands in for the app: answers each request line with `answer(request)`;
/// `None` closes the connection without an answer. Records every request.
fn fake_app(
    dir: &ShortDir,
    answer: impl Fn(&Value) -> Option<Value> + Send + 'static,
) -> Arc<Mutex<Vec<Value>>> {
    let listener = UnixListener::bind(dir.socket()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let mut out = stream.try_clone().unwrap();
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let req: Value = serde_json::from_str(&line).unwrap();
                log.lock().unwrap().push(req.clone());
                match answer(&req) {
                    Some(a) => writeln!(out, "{a}").unwrap(),
                    None => break,
                }
            }
        }
    });
    seen
}

fn adapter(dir: &ShortDir) -> Adapter {
    Adapter::new(Config {
        socket: dir.socket(),
        token: Some("gcapi_test".into()),
    })
}

fn call(a: &mut Adapter, tool: &str, arguments: Value) -> Value {
    let msg = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                      "params": { "name": tool, "arguments": arguments } });
    a.handle(&msg).unwrap()["result"].clone()
}

fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap().to_string()
}

#[test]
fn initialize_answers_the_asked_version_or_the_newest() {
    let mut a = Adapter::new(Config {
        socket: "/nonexistent".into(),
        token: None,
    });
    let init = |a: &mut Adapter, v: &str| {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                          "params": { "protocolVersion": v, "capabilities": {},
                                      "clientInfo": { "name": "t", "version": "1" } } });
        a.handle(&msg).unwrap()
    };
    let r = init(&mut a, "2025-03-26");
    assert_eq!(r["jsonrpc"], "2.0");
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(r["result"]["serverInfo"]["name"], "girocrab-mcp");
    assert_eq!(
        init(&mut a, "1999-01-01")["result"]["protocolVersion"],
        PROTOCOL_VERSIONS[0]
    );
}

#[test]
fn notifications_get_no_answer_and_unknown_requests_an_error() {
    let mut a = Adapter::new(Config {
        socket: "/nonexistent".into(),
        token: None,
    });
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert_eq!(a.handle(&note), None);
    let r = a
        .handle(&json!({ "jsonrpc": "2.0", "id": "x", "method": "resources/list" }))
        .unwrap();
    assert_eq!(r["error"]["code"], -32601);
    assert_eq!(r["id"], "x");
    let r = a
        .handle(&json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]))
        .unwrap();
    assert_eq!(r["error"]["code"], -32600, "no batches");
    let r = a
        .handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }))
        .unwrap();
    assert_eq!(r["result"], json!({}));
}

#[test]
fn tools_are_listed_read_only_with_object_schemas() {
    let mut a = Adapter::new(Config {
        socket: "/nonexistent".into(),
        token: None,
    });
    let r = a
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .unwrap();
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "get_info",
            "list_accounts",
            "list_categories",
            "list_transactions",
            "get_summary"
        ]
    );
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert_eq!(t["annotations"]["readOnlyHint"], true);
        assert!(!t["description"].as_str().unwrap().is_empty());
    }
}

#[test]
fn a_call_forwards_token_method_and_arguments() {
    let d = ShortDir::new();
    let seen = fake_app(&d, |_| {
        Some(json!({ "v": 1, "result": { "transactions": [], "next_cursor": null } }))
    });
    let mut a = adapter(&d);
    let r = call(
        &mut a,
        "list_transactions",
        json!({ "limit": 5, "direction": "income" }),
    );
    assert_eq!(r["isError"], false);
    assert_eq!(r["structuredContent"]["transactions"], json!([]));
    assert!(text(&r).contains("\"transactions\""));
    let req = seen.lock().unwrap()[0].clone();
    assert_eq!(
        req,
        json!({ "v": 1, "token": "gcapi_test", "method": "transactions",
                             "params": { "limit": 5, "direction": "income" } })
    );
}

#[test]
fn lists_are_wrapped_for_structured_content() {
    let d = ShortDir::new();
    fake_app(&d, |_| Some(json!({ "v": 1, "result": [{ "id": 1 }] })));
    let r = call(&mut adapter(&d), "list_accounts", Value::Null);
    assert_eq!(r["structuredContent"], json!({ "items": [{ "id": 1 }] }));
}

#[test]
fn refusals_become_readable_tool_errors() {
    for (code, expect) in [
        ("locked", "locked"),
        ("disabled", "turned off"),
        ("unauthorized", "access key"),
        ("not_shared", "not_shared: nope"),
    ] {
        let d = ShortDir::new();
        fake_app(&d, move |_| {
            Some(json!({ "v": 1, "error": { "code": code, "message": "nope" } }))
        });
        let r = call(&mut adapter(&d), "get_info", json!({}));
        assert_eq!(r["isError"], true, "{code}");
        assert!(text(&r).contains(expect), "{code}: {}", text(&r));
    }
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let mut a = Adapter::new(Config {
        socket: "/nonexistent".into(),
        token: None,
    });
    let msg =
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "pay" } });
    assert_eq!(a.handle(&msg).unwrap()["error"]["code"], -32602);
}

#[test]
fn without_key_or_app_the_user_is_told_what_to_do() {
    let mut a = Adapter::new(Config {
        socket: "/nonexistent/s.sock".into(),
        token: None,
    });
    assert!(text(&call(&mut a, "get_info", json!({}))).contains("GIROCRAB_API_TOKEN"));
    let mut a = Adapter::new(Config {
        socket: "/nonexistent/s.sock".into(),
        token: Some("k".into()),
    });
    let r = call(&mut a, "get_info", json!({}));
    assert_eq!(r["isError"], true);
    assert!(text(&r).contains("not reachable"), "{}", text(&r));
}

#[test]
fn a_connection_closed_by_the_app_is_reopened() {
    let d = ShortDir::new();
    // Every second request on a connection: closed without an answer (idle timeout).
    let n = Arc::new(Mutex::new(0));
    let seen = fake_app(&d, move |_| {
        let mut n = n.lock().unwrap();
        *n += 1;
        (*n % 2 == 1).then(|| json!({ "v": 1, "result": { "ok": *n } }))
    });
    let mut a = adapter(&d);
    assert_eq!(a.request("info", json!({})).unwrap(), json!({ "ok": 1 }));
    assert_eq!(a.request("info", json!({})).unwrap(), json!({ "ok": 3 }));
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "the lost request was sent again"
    );
}

#[test]
fn busy_is_waited_out() {
    let d = ShortDir::new();
    let n = Arc::new(Mutex::new(0));
    fake_app(&d, move |_| {
        let mut n = n.lock().unwrap();
        *n += 1;
        Some(if *n < 3 {
            json!({ "v": 1, "error": { "code": "busy", "message": "too many" } })
        } else {
            json!({ "v": 1, "result": {} })
        })
    });
    assert_eq!(adapter(&d).request("info", json!({})).unwrap(), json!({}));
}
