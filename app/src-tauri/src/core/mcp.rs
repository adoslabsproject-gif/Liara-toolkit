//! Minimal MCP (Model Context Protocol) host: connect to configured stdio MCP servers,
//! discover their tools and expose them to the agent as dynamic, consent-gated tools.
//! Config via the LIARA_MCP env var: a JSON array of {name, command, args}.
use crate::core::tools::{Tool, ToolSpec};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MCP_TIMEOUT: Duration = Duration::from_secs(20); // a mute server must not block forever

pub struct McpToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// Parse a `tools/list` result into tool specs (pure → unit-tested).
fn parse_tools(result: &Value) -> Vec<McpToolSpec> {
    result
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    Some(McpToolSpec {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
                        schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"})),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Flatten a `tools/call` result's content array into plain text (pure → unit-tested).
fn parse_call_result(result: &Value) -> String {
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if text.is_empty() {
        result.to_string()
    } else {
        text
    }
}

pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: i64,
}

impl McpClient {
    pub fn connect(command: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("avvio server MCP '{command}': {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        // background reader → channel, so request() can wait WITH a timeout (no hang on a mute server)
        let (tx, rx) = channel::<Value>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        let mut c = Self { child, stdin, rx, next_id: 1 };
        c.request(
            "initialize",
            json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "liara", "version": "1.0" } }),
        )?;
        c.notify("notifications/initialized", json!({}))?;
        Ok(c)
    }

    fn send(&mut self, msg: &Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Send a request and wait (with a timeout) for the reply with the matching id.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + MCP_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("server MCP: timeout ({}s)", MCP_TIMEOUT.as_secs()));
            }
            match self.rx.recv_timeout(remaining) {
                Ok(v) => {
                    if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                        if let Some(err) = v.get("error") {
                            return Err(anyhow!("MCP error: {err}"));
                        }
                        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                    }
                    // else: a notification or another id → keep waiting
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(anyhow!("server MCP: timeout ({}s)", MCP_TIMEOUT.as_secs()))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!("server MCP: connessione chiusa"))
                }
            }
        }
    }

    pub fn list_tools(&mut self) -> Result<Vec<McpToolSpec>> {
        Ok(parse_tools(&self.request("tools/list", json!({}))?))
    }

    pub fn call_tool(&mut self, name: &str, args: &Value) -> Result<String> {
        let res = self.request("tools/call", json!({ "name": name, "arguments": args }))?;
        Ok(parse_call_result(&res))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// A discovered MCP tool, exposed to the agent. Sensitive (external) → consent-gated.
pub struct McpTool {
    client: Arc<Mutex<McpClient>>,
    name: String,
    description: String,
    schema: Value,
}

impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: format!("mcp_{}", self.name),
            description: self.description.clone(),
            parameters: self.schema.clone(),
        }
    }
    fn execute(&self, args: &Value) -> Result<String> {
        self.client.lock().unwrap().call_tool(&self.name, args)
    }
    fn sensitive(&self) -> bool {
        true
    }
    fn consent_action(&self, _args: &Value) -> String {
        format!("usare lo strumento esterno MCP «{}»", self.name)
    }
}

/// Connect to every server in the LIARA_MCP env (JSON array). Failures are skipped, not fatal.
pub fn connect_configured() -> Vec<Box<dyn Tool>> {
    let Ok(cfg) = std::env::var("LIARA_MCP") else { return Vec::new() };
    let Ok(servers) = serde_json::from_str::<Vec<Value>>(&cfg) else {
        eprintln!("LIARA_MCP: JSON non valido");
        return Vec::new();
    };
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    for s in servers {
        let command = s.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let args: Vec<String> = s
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        match McpClient::connect(command, &args) {
            Ok(mut client) => match client.list_tools() {
                Ok(specs) => {
                    let shared = Arc::new(Mutex::new(client));
                    for sp in specs {
                        tools.push(Box::new(McpTool {
                            client: shared.clone(),
                            name: sp.name,
                            description: sp.description,
                            schema: sp.schema,
                        }));
                    }
                }
                Err(e) => eprintln!("MCP list_tools '{command}': {e}"),
            },
            Err(e) => eprintln!("MCP '{command}': {e}"),
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tools_list() {
        let res = json!({ "tools": [
            { "name": "read_file", "description": "Read a file", "inputSchema": { "type": "object" } },
            { "name": "write_file" }
        ]});
        let tools = parse_tools(&res);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(tools[0].description, "Read a file");
        assert_eq!(tools[1].name, "write_file"); // missing description → empty, still parsed
    }

    #[test]
    fn flattens_call_result_text() {
        let res = json!({ "content": [ { "type": "text", "text": "riga 1" }, { "type": "text", "text": "riga 2" } ] });
        assert_eq!(parse_call_result(&res), "riga 1\nriga 2");
    }

    #[test]
    fn empty_config_no_tools() {
        std::env::remove_var("LIARA_MCP");
        assert!(connect_configured().is_empty());
    }
}
