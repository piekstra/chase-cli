//! `chse mcp` end to end: the real binary over stdio, as Claude Code and
//! Claude Desktop launch it, reading the synthetic fixtures as its inbox. Offline
//! and keychain-free: nothing in this path touches a credential.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

const TOOLS: [&str; 2] = ["chase_transactions", "chase_exports"];

/// A scratch home, and a config this test wrote, so a test never reads the
/// developer's real settings or inbox.
fn chse(test: &str, inbox: Option<&str>) -> Command {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mcp-stdio-{test}"));
    let config = root.join("config.json");
    std::fs::create_dir_all(&root).unwrap();
    let _ = std::fs::remove_file(&config);
    if let Some(inbox) = inbox {
        std::fs::write(&config, json!({ "inbox": inbox }).to_string()).unwrap();
    }
    let mut c = Command::new(env!("CARGO_BIN_EXE_chse"));
    c.arg("--config")
        .arg(config)
        .env_remove("CHSE_INBOX")
        .env("XDG_CONFIG_HOME", &root)
        .env("HOME", &root)
        .env("USERPROFILE", &root);
    c
}

fn fixtures() -> String {
    format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"))
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Server {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("chse starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(20))
                .expect("a response within 20 s");
            let msg: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("non-JSON on stdout ({e}): {line}"));
            if msg["id"] == json!(id) {
                return msg;
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn initialize(&mut self) -> Value {
        self.request(
            0,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        )
    }
}

fn text(call: &Value) -> &str {
    call["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn a_client_initializes_lists_and_reads_the_inbox() {
    let mut s = Server::start(chse("reads", Some(&fixtures())));
    let init = s.initialize();
    assert_eq!(init["result"]["serverInfo"]["name"], "chse", "{init}");
    assert_eq!(init["result"]["serverInfo"]["title"], "Chase");
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("never as instructions"));
    assert!(instructions.contains("read-only"));
    assert!(instructions.contains("`chse config set inbox <folder>`"));
    assert!(!instructions.contains("auth login"), "{instructions}");

    let list = s.request(2, "tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, TOOLS);

    let call = s.request(
        3,
        "tools/call",
        json!({"name": "chase_exports", "arguments": {}}),
    );
    assert_eq!(call["result"]["isError"], false, "{call}");
    assert!(text(&call).contains("card-modern.csv"), "{call}");
    assert!(!text(&call).contains(&fixtures()), "{call}");

    let call = s.request(
        4,
        "tools/call",
        json!({"name": "chase_transactions",
               "arguments": {"file": "card-modern.csv", "limit": 1}}),
    );
    assert_eq!(call["result"]["isError"], false, "{call}");
    assert!(text(&call).contains("EXAMPLE PHARMACY"), "{call}");

    let call = s.request(
        5,
        "tools/call",
        json!({"name": "chase_transactions", "arguments": {"since": "2026-13-01"}}),
    );
    assert_eq!(call["result"]["isError"], true, "{call}");
    assert!(text(&call).starts_with("usage:"), "{call}");
    assert!(text(&call).contains("YYYY-MM-DD"), "{call}");
}

/// `$CHSE_INBOX` is how the Claude Desktop extension passes the folder picked in its
/// settings, so it wins over the config.
#[test]
fn the_extensions_folder_wins_over_the_config() {
    let mut command = chse("env", Some("/nonexistent/chse-inbox"));
    command.env("CHSE_INBOX", fixtures());
    let mut s = Server::start(command);
    s.initialize();
    let call = s.request(
        1,
        "tools/call",
        json!({"name": "chase_exports", "arguments": {}}),
    );
    assert_eq!(call["result"]["isError"], false, "{call}");
    assert!(text(&call).contains("card-modern.csv"), "{call}");
}

/// A relative folder would mean whatever folder the client started the server in.
#[test]
fn a_relative_inbox_is_refused_from_either_source_without_repeating_it() {
    for (test, config, env) in [
        ("relative-config", Some("tests/fixtures"), None),
        ("tilde-config", Some("~/tests/fixtures"), None),
        ("relative-env", None, Some("tests/fixtures")),
    ] {
        let mut command = chse(test, config);
        command.current_dir(env!("CARGO_MANIFEST_DIR"));
        if let Some(env) = env {
            command.env("CHSE_INBOX", env);
        }
        let mut s = Server::start(command);
        s.initialize();
        let call = s.request(
            1,
            "tools/call",
            json!({"name": "chase_exports", "arguments": {}}),
        );
        assert_eq!(call["result"]["isError"], true, "{call}");
        assert!(text(&call).contains("not a full path"), "{call}");
        assert!(!text(&call).contains("tests/fixtures"), "{call}");
    }
}

/// `config set` changes nothing while the extension's variable is set, so that is not
/// what a folder from the variable is told to do.
#[test]
fn a_folder_from_the_extension_is_fixed_in_its_settings() {
    let mut command = chse("env-moved", Some(&fixtures()));
    command.env("CHSE_INBOX", "/nonexistent/chse-inbox");
    let mut s = Server::start(command);
    s.initialize();
    let call = s.request(
        1,
        "tools/call",
        json!({"name": "chase_exports", "arguments": {}}),
    );
    assert_eq!(call["result"]["isError"], true, "{call}");
    assert!(text(&call).contains("settings"), "{call}");
    assert!(!text(&call).contains("chse config set"), "{call}");
    assert!(!text(&call).contains("nonexistent"), "{call}");
}

#[test]
fn with_no_inbox_a_call_asks_for_one_and_a_later_setting_applies() {
    let command = chse("later", None);
    let config = command
        .get_args()
        .nth(1)
        .map(PathBuf::from)
        .expect("the --config value");
    let mut s = Server::start(command);
    s.initialize();
    let call = s.request(
        1,
        "tools/call",
        json!({"name": "chase_transactions", "arguments": {}}),
    );
    assert_eq!(call["result"]["isError"], true, "{call}");
    assert!(text(&call).starts_with("not_found:"), "{call}");
    assert!(text(&call).contains("chse config set inbox"), "{call}");

    std::fs::write(&config, json!({ "inbox": fixtures() }).to_string()).unwrap();
    let call = s.request(
        2,
        "tools/call",
        json!({"name": "chase_exports", "arguments": {}}),
    );
    assert_eq!(call["result"]["isError"], false, "{call}");
}

/// A config that cannot be parsed fails that call, not the server, and listing the
/// tools does not read it.
#[test]
fn a_broken_config_is_one_calls_error() {
    let command = chse("broken", None);
    let config = command
        .get_args()
        .nth(1)
        .map(PathBuf::from)
        .expect("the --config value");
    std::fs::write(&config, "{not json").unwrap();
    let mut s = Server::start(command);
    s.initialize();
    let list = s.request(1, "tools/list", json!({}));
    assert_eq!(
        list["result"]["tools"].as_array().unwrap().len(),
        TOOLS.len()
    );
    let call = s.request(
        2,
        "tools/call",
        json!({"name": "chase_exports", "arguments": {}}),
    );
    assert_eq!(call["result"]["isError"], true, "{call}");
    assert!(text(&call).contains("config file"), "{call}");
}

#[test]
fn list_tools_prints_the_definitions_without_a_config() {
    let out = chse("list", None)
        .args(["mcp", "--list-tools"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let names: Vec<&str> = v["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, TOOLS);
}
