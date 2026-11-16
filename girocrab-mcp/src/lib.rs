//! MCP adapter for GiroCrab's read-only local interface (#136, PoC part 3).
//!
//! An MCP client starts `girocrab-mcp` and speaks JSON-RPC 2.0 over stdio, one
//! message per line. Each tool call becomes one request on the socket the app
//! serves (`docs/local-api.md`). This program never opens the vault: it only
//! forwards, and whatever the socket refuses (locked, turned off, wrong key)
//! comes back as a tool error the assistant can explain to the user.
//!
//! Only the subset of MCP a read-only tool server needs: `initialize`, `ping`,
//! `tools/list`, `tools/call`; notifications are accepted and ignored.

use serde_json::{json, Map, Value};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// MCP revisions this adapter speaks, newest first. A client asking for one of
/// them gets it; any other request is answered with the newest.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Version of the socket protocol (`"v"` of every request).
const API_VERSION: u32 = 1;
/// `busy` (all connections of the app in use): tries after the first.
const BUSY_RETRIES: u32 = 3;
const BUSY_WAIT: Duration = Duration::from_millis(250);
/// One request on the socket, also while the app paces requests.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Config {
    pub socket: PathBuf,
    /// The access key from GiroCrab (Settings › Analysis), or why there is none
    /// (told to the user on every tool call instead of refusing to start).
    pub token: Result<String, String>,
}

/// The message for a missing key.
pub fn no_token() -> String {
    format!(
        "No access key configured. Create one in GiroCrab ({SETTINGS}) and set \
         GIROCRAB_API_TOKEN (or --token-file) in this program's configuration."
    )
}

/// One MCP tool and the socket method behind it.
struct Tool {
    name: &'static str,
    method: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "get_info",
        method: "info",
        description: "What GiroCrab shares: API version, number of shared accounts, which \
            text fields are shared, and the time of the last bank sync (Unix seconds).",
        schema: no_arguments,
    },
    Tool {
        name: "list_accounts",
        method: "accounts",
        description: "The shared accounts with name, bank, kind, currency, balance in cents \
            (balance_cents) and its date, and the last sync. IBAN and account number only \
            when the user shares them.",
        schema: no_arguments,
    },
    Tool {
        name: "list_categories",
        method: "categories",
        description: "The category tree: id, parent_id, name, full path (\"Housing › Rent\") \
            and depth. Bookings refer to categories by id.",
        schema: no_arguments,
    },
    Tool {
        name: "list_transactions",
        method: "transactions",
        description: "Bookings of the shared accounts, newest first. Amounts in cents \
            (amount_cents, negative = outgoing), dates as YYYY-MM-DD, stable ids. Pass \
            next_cursor back as cursor for the next page; no next_cursor = last page. Text \
            fields the user does not share are missing; IBANs and long numbers in texts are \
            masked unless account numbers are shared. search needs payees, purposes and \
            account numbers to be shared.",
        schema: transaction_schema,
    },
    Tool {
        name: "get_summary",
        method: "summary",
        description: "Totals for a date range of at most three years, calculated by GiroCrab \
            (amounts in cents). totals: income and expenses per currency like the app's \
            tiles (transfers between own accounts are neutral, refunds reduce expenses). \
            by_category: net sum per category like the app's category list (transfers and \
            loan payments included).",
        schema: summary_schema,
    },
];

fn no_arguments() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn date(description: &str) -> Value {
    json!({ "type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$", "description": description })
}

fn account_ids() -> Value {
    json!({
        "type": "array", "items": { "type": "integer" },
        "description": "Only these shared accounts (default: all shared accounts)"
    })
}

fn transaction_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "account_ids": account_ids(),
            "date_from": date("First booking date, inclusive"),
            "date_to": date("Last booking date, inclusive"),
            "category_id": { "type": "integer", "description": "Only this category" },
            "include_subcategories": {
                "type": "boolean", "description": "With category_id: also its subcategories (default true)"
            },
            "uncategorized_only": { "type": "boolean", "description": "Only bookings without a category" },
            "direction": { "type": "string", "enum": ["income", "expense", "all"] },
            "search": { "type": "string", "description": "Text in payee, purpose, booking text, IBANs or account numbers" },
            "cursor": { "type": "string", "description": "next_cursor of the previous page" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Page size (default 100)" }
        },
        "additionalProperties": false
    })
}

fn summary_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "date_from": date("First day, inclusive"),
            "date_to": date("Last day, inclusive (at most three years after date_from)"),
            "account_ids": account_ids()
        },
        "required": ["date_from", "date_to"],
        "additionalProperties": false
    })
}

/// A failed tool call, worded for the user; sent as a tool result with `isError`.
#[derive(Debug, PartialEq)]
pub struct ToolError(pub String);

const SETTINGS: &str = "Settings › Analysis, German: Einstellungen › Analyse";

/// The socket answer of a refused request, as something the user can act on.
fn api_error(code: &str, message: &str) -> ToolError {
    ToolError(match code {
        "locked" => "GiroCrab is locked. Unlock it to continue.".into(),
        "disabled" => format!("External analysis is turned off in GiroCrab ({SETTINGS})."),
        "unauthorized" => format!(
            "GiroCrab did not accept the access key. Create a new key in GiroCrab \
             ({SETTINGS}) and update this program's configuration."
        ),
        "busy" => "GiroCrab is busy with other programs. Try again shortly.".into(),
        _ => format!("{code}: {message}"),
    })
}

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

/// Why one exchange on a connection brought no answer.
enum Lost {
    /// Closed before an answer (idle timeout of the app, app gone): worth one
    /// new connection.
    Closed,
    /// Anything else: told to the user as it is, no repeat.
    Failed(ToolError),
}

impl Conn {
    fn open(config: &Config) -> Result<Self, ToolError> {
        let socket = config.socket.display();
        let stream = UnixStream::connect(&config.socket).map_err(|e| match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => ToolError(format!(
                "GiroCrab is not reachable: it is not running, the vault is locked, or \
                 external analysis is turned off ({SETTINGS}). Socket: {socket}"
            )),
            ErrorKind::PermissionDenied => ToolError(format!(
                "GiroCrab's socket cannot be opened (permission denied): it belongs to \
                 another user, or --socket points to the wrong place. Socket: {socket}"
            )),
            // sun_path holds 104 bytes on macOS (108 on Linux).
            ErrorKind::InvalidInput => ToolError(format!(
                "The socket path is too long for a Unix socket: {socket}"
            )),
            _ => ToolError(format!("Cannot connect to GiroCrab ({socket}): {e}")),
        })?;
        let io = |e: std::io::Error| ToolError(format!("Cannot connect to GiroCrab: {e}"));
        // Fails (EINVAL on macOS) only on a connection the app already closed,
        // e.g. right after answering `busy`: the read below then returns that
        // answer or EOF at once, so a missing timeout cannot block.
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        Ok(Self {
            writer: stream.try_clone().map_err(io)?,
            reader: BufReader::new(stream),
        })
    }

    /// One request line, one answer line. The answer is read even when the
    /// write failed: a refused connection (`busy`) carries its answer although
    /// the app closed it without reading the request.
    fn round_trip(&mut self, line: &str) -> Result<Value, Lost> {
        let written = writeln!(self.writer, "{line}");
        let mut answer = String::new();
        match self.reader.read_line(&mut answer) {
            Ok(n) if n > 0 => serde_json::from_str(&answer).map_err(|e| {
                Lost::Failed(ToolError(format!(
                    "GiroCrab sent an unreadable answer: {e}"
                )))
            }),
            Ok(_) => Err(Lost::Closed),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Err(Lost::Failed(ToolError(format!(
                    "GiroCrab did not answer within {} seconds.",
                    IO_TIMEOUT.as_secs()
                ))))
            }
            Err(_) if written.is_err() => Err(Lost::Closed),
            Err(e) => Err(Lost::Failed(ToolError(format!(
                "Connection to GiroCrab failed: {e}"
            )))),
        }
    }
}

pub struct Adapter {
    config: Config,
    /// Kept between calls; the app closes it after a minute without requests.
    conn: Option<Conn>,
    /// Negotiated in `initialize`; decides whether results carry `structuredContent`.
    protocol: &'static str,
}

impl Adapter {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            conn: None,
            protocol: PROTOCOL_VERSIONS[0],
        }
    }

    /// Answers one JSON-RPC message; None for notifications.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            // Batches were dropped from MCP in 2025-06-18.
            return Some(rpc_error(
                Value::Null,
                -32600,
                "expected one JSON-RPC object",
            ));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A response (result or error) is never answered; we send no
            // requests, so it is dropped. Anything else is no valid message.
            if obj.contains_key("result") || obj.contains_key("error") {
                return None;
            }
            return id.map(|id| rpc_error(id, -32600, "invalid request"));
        };
        let id = id?; // notifications (initialized, cancelled, …) need no answer
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let result = initialize(&params);
                self.protocol = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| result["protocolVersion"] == **v)
                    .copied()
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                rpc_result(id, result)
            }
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(id, tools_list()),
            "tools/call" => match self.call_tool(&params) {
                Ok(result) => rpc_result(id, result),
                Err((code, message)) => rpc_error(id, code, &message),
            },
            _ => rpc_error(id, -32601, &format!("method not found: {method}")),
        })
    }

    /// A tool result (also for refused requests); Err only for a call that
    /// names no known tool.
    fn call_tool(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let tool = TOOLS
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| (-32602, format!("unknown tool: {name}")))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(a) => a.clone(),
        };
        Ok(match self.request(tool.method, arguments) {
            Ok(result) => {
                // Compact: the text block is what most clients hand to the model.
                let text = result.to_string();
                let mut out =
                    json!({ "content": [{ "type": "text", "text": text }], "isError": false });
                // Structured content exists from 2025-06-18 on (dates compare as text).
                if self.protocol >= "2025-06-18" {
                    // Lists are wrapped: structured content must be an object.
                    out["structuredContent"] = match result {
                        Value::Object(_) => result,
                        other => json!({ "items": other }),
                    };
                }
                out
            }
            Err(ToolError(text)) => {
                json!({ "content": [{ "type": "text", "text": text }], "isError": true })
            }
        })
    }

    /// One request on the socket: reconnects once when the kept connection was
    /// closed (read-only, so a repeat is harmless) and waits out `busy`.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        let token = self
            .config
            .token
            .as_deref()
            .map_err(|e| ToolError(e.clone()))?;
        let line = json!({ "v": API_VERSION, "token": token, "method": method, "params": params })
            .to_string();
        for attempt in 0..=BUSY_RETRIES {
            let answer = self.round_trip(&line)?;
            match answer.get("error") {
                None => return Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
                Some(err) => {
                    let code = err
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("internal");
                    if code == "busy" && attempt < BUSY_RETRIES {
                        self.conn = None; // the app closes a refused connection
                        std::thread::sleep(BUSY_WAIT);
                        continue;
                    }
                    let message = err.get("message").and_then(Value::as_str).unwrap_or("");
                    return Err(api_error(code, message));
                }
            }
        }
        unreachable!("the last attempt returns")
    }

    /// On the kept connection, or a new one when the kept one was closed (only
    /// then is the request sent again; read-only, so that is harmless).
    fn round_trip(&mut self, line: &str) -> Result<Value, ToolError> {
        if let Some(conn) = self.conn.as_mut() {
            match conn.round_trip(line) {
                Ok(answer) => return Ok(answer),
                Err(Lost::Closed) => {}
                Err(Lost::Failed(e)) => {
                    self.conn = None;
                    return Err(e);
                }
            }
        }
        self.conn = None;
        let mut conn = Conn::open(&self.config)?;
        match conn.round_trip(line) {
            Ok(answer) => {
                self.conn = Some(conn);
                Ok(answer)
            }
            Err(Lost::Closed) => Err(ToolError(
                "GiroCrab closed the connection without an answer.".into(),
            )),
            Err(Lost::Failed(e)) => Err(e),
        }
    }
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "girocrab-mcp", "title": "GiroCrab", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Read-only access to the accounts, bookings, categories and totals the \
            user shares in GiroCrab. Nothing can be changed or paid. Amounts are in cents. \
            If a tool reports that GiroCrab is locked or not reachable, ask the user to \
            unlock or start GiroCrab."
    })
}

fn tools_list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
                "annotations": { "readOnlyHint": true, "openWorldHint": false }
            })
        })
        .collect();
    json!({ "tools": tools })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
