use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static MSG_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    MSG_ID.fetch_add(1, Ordering::SeqCst)
}

pub struct McpServer {
    pub name: String,
    command: String,
    args: Vec<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    reader: Option<BufReader<ChildStdout>>,
    pub tools: Vec<Value>,
    /// Signature status of the *package* this server was declared in, from
    /// `cmd::sign::describe_signature` — attributed at load time so it's
    /// correct even when several packages are merged into one McpManager
    /// (see `serve.rs`'s multi-package fold).
    pub signed_as: String,
}

impl McpServer {
    pub fn new(name: &str, command: &str, args: Vec<String>, signed_as: &str) -> Self {
        Self {
            name: name.to_string(),
            command: command.to_string(),
            args,
            child: None,
            stdin: None,
            reader: None,
            tools: Vec::new(),
            signed_as: signed_as.to_string(),
        }
    }

    pub fn command_line(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args.join(" "))
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout"))?;

        self.stdin = Some(stdin);
        self.reader = Some(BufReader::new(stdout));
        self.child = Some(child);

        self.initialize()?;
        self.tools = self.list_tools()?;
        Ok(())
    }

    fn send(&mut self, msg: &Value) -> Result<()> {
        let line = serde_json::to_string(msg)? + "\n";
        self.stdin.as_mut().unwrap().write_all(line.as_bytes())?;
        self.stdin.as_mut().unwrap().flush()?;
        Ok(())
    }

    fn recv_with_id(&mut self, id: u64, timeout: Duration) -> Result<Value> {
        let deadline = Instant::now() + timeout;
        let reader = self.reader.as_mut().unwrap();
        let mut line = String::new();

        loop {
            if Instant::now() > deadline {
                bail!("MCP timeout waiting for id={id}");
            }
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => bail!("MCP server closed"),
                Ok(_) => {
                    let msg: Value = serde_json::from_str(line.trim())?;
                    if msg["id"] == id {
                        return Ok(msg);
                    }
                    // skip notifications
                }
                Err(e) => bail!("MCP read error: {e}"),
            }
        }
    }

    fn initialize(&mut self) -> Result<()> {
        let id = next_id();
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "id": id,
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "aipk", "version": "0.1"}
            }
        }))?;
        let resp = self.recv_with_id(id, Duration::from_secs(15))?;
        if resp["result"].is_object() {
            self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        }
        Ok(())
    }

    fn list_tools(&mut self) -> Result<Vec<Value>> {
        let id = next_id();
        self.send(&json!({"jsonrpc":"2.0","method":"tools/list","id":id,"params":{}}))?;
        let resp = self.recv_with_id(id, Duration::from_secs(10))?;
        Ok(resp["result"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    pub fn call(&mut self, tool_name: &str, arguments: &Value) -> Result<String> {
        let id = next_id();
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": id,
            "params": {"name": tool_name, "arguments": arguments}
        }))?;
        let resp = self.recv_with_id(id, Duration::from_secs(60))?;
        if let Some(err) = resp["error"].as_object() {
            return Ok(format!(
                "[MCP error: {}]",
                err.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
            ));
        }
        let content = &resp["result"]["content"];
        let parts: Vec<String> = content
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|item| match item["type"].as_str() {
                Some("text") => item["text"].as_str().unwrap_or("").to_string(),
                Some("image") => "[image result]".to_string(),
                _ => serde_json::to_string(item).unwrap_or_default(),
            })
            .collect();
        Ok(if parts.is_empty() {
            "[empty result]".to_string()
        } else {
            parts.join("\n")
        })
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct McpManager {
    pub servers: Vec<McpServer>,
    /// Namespaced `"{server}__{tool}"` entries from `--allow-tool <server>:<tool>`.
    /// Empty = no restriction beyond `confirm_launch`'s server-level consent
    /// (today's behavior, unchanged by default).
    allowed_tools: std::collections::HashSet<String>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
            allowed_tools: std::collections::HashSet::new(),
        }
    }

    /// `allowed` entries are `"server:tool"` as typed on the CLI; converted
    /// here to the `"server__tool"` namespacing `call_tool`/`to_openai_tools`
    /// already use internally.
    pub fn with_allowed_tools(mut self, allowed: &[String]) -> Self {
        self.allowed_tools = allowed.iter().map(|s| s.replacen(':', "__", 1)).collect();
        self
    }

    pub fn load_from_pkg(mut self, pkg: &crate::format::AipkPackage) -> Self {
        if let Some(tools) = pkg.tools_json() {
            if let Some(servers) = tools["mcp_servers"].as_array() {
                let signed_as = crate::cmd::sign::describe_signature(pkg);
                for s in servers {
                    let command = s["command"].as_str().unwrap_or("").to_string();
                    if command.is_empty() {
                        continue;
                    }
                    let args: Vec<String> = s["args"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|a| a.as_str().map(|s| s.to_string()))
                        .collect();
                    let name = s["name"].as_str().unwrap_or("server").to_string();
                    self.servers
                        .push(McpServer::new(&name, &command, args, &signed_as));
                }
            }
        }
        self
    }

    /// Print the MCP servers this package would launch — with each server's
    /// signature status (see `McpServer::signed_as` / `cmd::sign::describe_signature`;
    /// this shows integrity/identity, it is not itself a trust decision) — and
    /// gate on the caller's consent before any process is spawned. Packages
    /// declare arbitrary shell commands here (e.g. `npx ...`), so this is the
    /// only checkpoint between loading an untrusted `.aipk` file and it running
    /// code on this machine.
    ///
    /// `allowed` pre-approves specific servers by name (`--allow <name>`,
    /// repeatable) without a blanket `--yes` — if every declared server is
    /// covered by `allowed` (or `trust_all`), the interactive prompt is
    /// skipped entirely. Otherwise the remaining servers still get one
    /// combined y/N, same as before this existed (per-server prompts would be
    /// its own UX regression — not attempted here).
    ///
    /// Returns `true` if it's safe to proceed to `start_all()`.
    pub fn confirm_launch(&self, trust_all: bool, allowed: &[String]) -> bool {
        if self.servers.is_empty() {
            return true;
        }
        let is_allowed = |name: &str| trust_all || allowed.iter().any(|a| a == name);
        let all_allowed = self.servers.iter().all(|s| is_allowed(&s.name));

        eprintln!(
            "This package declares {} MCP server(s) that will run as local processes:",
            self.servers.len()
        );
        for srv in &self.servers {
            let pre_approved = if is_allowed(&srv.name) {
                " [pre-approved]"
            } else {
                ""
            };
            eprintln!(
                "  - {}: {} [{}]{}",
                srv.name,
                srv.command_line(),
                srv.signed_as,
                pre_approved
            );
        }
        if trust_all {
            eprintln!("Trusting (--yes). No sandboxing is applied — these run with your full user permissions.");
            return true;
        }
        if all_allowed {
            eprintln!("All servers pre-approved via --allow. No sandboxing is applied — these run with your full user permissions.");
            return true;
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!(
                "Refusing to launch MCP servers non-interactively. Re-run with --yes, or --allow <name> for each server, if you trust this package."
            );
            return false;
        }
        eprint!("Run these commands with your full user permissions? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return false;
        }
        matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    }

    pub fn start_all(&mut self) -> usize {
        let mut count = 0;
        for srv in &mut self.servers {
            match srv.start() {
                Ok(_) => count += 1,
                Err(e) => eprintln!("MCP '{}' failed: {e}", srv.name),
            }
        }
        count
    }

    pub fn to_openai_tools(&self) -> Vec<Value> {
        let mut result = Vec::new();
        for srv in &self.servers {
            for tool in &srv.tools {
                let fn_name = format!("{}__{}", srv.name, tool["name"].as_str().unwrap_or(""));
                result.push(json!({
                    "type": "function",
                    "function": {
                        "name": fn_name,
                        "description": tool["description"].as_str().unwrap_or(""),
                        "parameters": tool.get("inputSchema").cloned().unwrap_or(json!({"type":"object","properties":{}}))
                    }
                }));
            }
        }
        result
    }

    pub fn call_tool(&mut self, namespaced: &str, args: &Value) -> String {
        if !self.allowed_tools.is_empty() && !self.allowed_tools.contains(namespaced) {
            return format!("[tool blocked by allowlist: {namespaced}]");
        }
        let (server_name, tool_name) = if let Some(pos) = namespaced.find("__") {
            (&namespaced[..pos], &namespaced[pos + 2..])
        } else {
            ("", namespaced)
        };

        for srv in &mut self.servers {
            if !server_name.is_empty() && srv.name != server_name {
                continue;
            }
            if srv
                .tools
                .iter()
                .any(|t| t["name"].as_str() == Some(tool_name))
            {
                return srv
                    .call(tool_name, args)
                    .unwrap_or_else(|e| format!("[error: {e}]"));
            }
        }
        format!("[tool not found: {namespaced}]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_launch_true_when_no_servers() {
        let mgr = McpManager::new();
        assert!(mgr.confirm_launch(false, &[]));
    }

    #[test]
    fn confirm_launch_true_when_trust_all() {
        let mut mgr = McpManager::new();
        mgr.servers.push(McpServer::new(
            "srv",
            "echo",
            vec![],
            "UNSIGNED — origin cannot be verified",
        ));
        assert!(mgr.confirm_launch(true, &[]));
    }

    #[test]
    fn confirm_launch_true_when_all_servers_allowed() {
        let mut mgr = McpManager::new();
        mgr.servers.push(McpServer::new(
            "a",
            "echo",
            vec![],
            "UNSIGNED — origin cannot be verified",
        ));
        mgr.servers.push(McpServer::new(
            "b",
            "echo",
            vec![],
            "UNSIGNED — origin cannot be verified",
        ));
        assert!(mgr.confirm_launch(false, &["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn call_tool_blocked_when_not_in_allowlist() {
        let mgr = McpManager::new().with_allowed_tools(&["srv:ok_tool".to_string()]);
        let mut mgr = mgr;
        let result = mgr.call_tool("srv__other_tool", &json!({}));
        assert_eq!(result, "[tool blocked by allowlist: srv__other_tool]");
    }

    #[test]
    fn call_tool_allowlist_empty_means_unrestricted() {
        // Empty allowlist should fall through to the normal "not found" path
        // rather than being blocked — no server registered, so it's not found,
        // but crucially NOT "[tool blocked ...]".
        let mut mgr = McpManager::new();
        let result = mgr.call_tool("srv__any_tool", &json!({}));
        assert_eq!(result, "[tool not found: srv__any_tool]");
    }
}
