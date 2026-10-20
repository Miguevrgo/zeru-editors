//! The server driven as an editor drives it, over stdin and stdout. The
//! compiler is `zeru` from `ZERU_PATH` or `PATH`, behind a script that makes
//! each check take a second, so a request sent during one shows it does not
//! wait for it.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const MAIN: &str = "import util;

struct Point {
    x: i32,
    y: i32,

    fn sum(self) i32 {
        return self.x + self.y;
    }

    fn origin() Point {
        return Point { x: 0, y: 0 };
    }
}

// A comment.
fn main() {
    var p = Point { x: 1, y: 2 };
    var q = Point.origin();
    var v: Vec<i32> = Vec.new();
    v.push(util::twice(p.sum()));
    println(\"{}\", q.x);
}
";

const UTIL: &str = "// Twice `n`.\npub fn twice(n: i32) i32 {\n    return n * 2;\n}\n";

struct Server {
    child: Child,
    input: ChildStdin,
    output: Receiver<Value>,
    next: i64,
    dir: PathBuf,
}

impl Server {
    fn start() -> Self {
        let dir = std::env::temp_dir().join(format!("zeru-lsp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.zr"), MAIN).unwrap();
        std::fs::write(dir.join("util.zr"), UTIL).unwrap();
        let zeru = std::env::var("ZERU_PATH").unwrap_or_else(|_| "zeru".into());
        let slow = dir.join("slow-zeru");
        std::fs::write(&slow, format!("#!/bin/sh\nsleep 1\nexec '{zeru}' \"$@\"\n")).unwrap();
        Command::new("chmod").arg("+x").arg(&slow).status().unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_zeru-lsp"))
            .env("ZERU_PATH", &slow)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("zeru-lsp starts");
        let input = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (sender, output) = channel();
        std::thread::spawn(move || {
            loop {
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if stdout.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    match line.trim().strip_prefix("Content-Length: ") {
                        Some(value) => length = value.parse().unwrap(),
                        None if line.trim().is_empty() => break,
                        None => {}
                    }
                }
                let mut body = vec![0; length];
                stdout.read_exact(&mut body).unwrap();
                if sender.send(serde_json::from_slice(&body).unwrap()).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            input,
            output,
            next: 0,
            dir,
        }
    }

    fn uri(&self, name: &str) -> String {
        format!("file://{}", self.dir.join(name).display())
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.input, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.input.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(20))
            .expect("the server answers")
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn ask(&mut self, method: &str, params: Value) -> i64 {
        self.next += 1;
        let id = self.next;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.ask(method, params);
        loop {
            let message = self.receive();
            if message["id"] == id {
                return message["result"].clone();
            }
        }
    }

    /// The next diagnostics published for `name`.
    fn diagnostics(&mut self, name: &str) -> Vec<Value> {
        let uri = self.uri(name);
        loop {
            let message = self.receive();
            if message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
            {
                return message["params"]["diagnostics"].as_array().unwrap().clone();
            }
        }
    }

    fn at(&mut self, method: &str, line: u32, character: u32) -> Value {
        let mut params = json!({
            "textDocument": { "uri": self.uri("main.zr") },
            "position": { "line": line, "character": character },
        });
        match method {
            "textDocument/references" => params["context"] = json!({ "includeDeclaration": true }),
            "textDocument/rename" => params["newName"] = json!("point"),
            _ => {}
        }
        self.request(method, params)
    }

    fn change(&mut self, text: &str) {
        let uri = self.uri("main.zr");
        let changes = json!([{ "text": text }]);
        self.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": changes }),
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        self.child.wait().unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn labels(found: &Value) -> Vec<&str> {
    let found = found["items"].as_array().unwrap().iter();
    found.map(|item| item["label"].as_str().unwrap()).collect()
}

fn shown(hover: &Value) -> &str {
    hover["contents"]["value"].as_str().unwrap()
}

#[test]
fn answers_and_checks_without_blocking() {
    let mut server = Server::start();
    let initialized = server.request("initialize", json!({ "capabilities": { "textDocument": { "completion": { "completionItem": { "snippetSupport": true } } } } }));
    let legend =
        initialized["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"].clone();
    server.notify("initialized", json!({}));
    let document =
        json!({ "uri": server.uri("main.zr"), "languageId": "zeru", "version": 1, "text": MAIN });
    server.notify("textDocument/didOpen", json!({ "textDocument": document }));
    assert_eq!(server.diagnostics("main.zr"), Vec::<Value>::new());
    assert_eq!(server.diagnostics("main.zr"), Vec::<Value>::new());

    assert!(shown(&server.at("textDocument/hover", 20, 23)).contains("var p: Point"));
    assert!(shown(&server.at("textDocument/hover", 20, 26)).contains("Point\nfn sum(self) i32"));
    assert_eq!(
        shown(&server.at("textDocument/hover", 20, 17)),
        "```zeru\npub fn twice(n: i32) i32\n```\n\n---\n\nTwice `n`."
    );
    assert!(shown(&server.at("textDocument/hover", 19, 8)).contains("var v: Vec<i32>"));

    let range =
        json!({ "start": { "line": 0, "character": 0 }, "end": { "line": 30, "character": 0 } });
    let uri = server.uri("main.zr");
    let hints = server.request(
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": uri }, "range": range }),
    );
    let hints: Vec<(u64, u64, &str)> = hints
        .as_array()
        .unwrap()
        .iter()
        .map(|hint| {
            let at = &hint["position"];
            (
                at["line"].as_u64().unwrap(),
                at["character"].as_u64().unwrap(),
                hint["label"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(hints, [(17, 9, ": Point"), (18, 9, ": Point")]);

    let definition = server.at("textDocument/definition", 20, 26);
    assert_eq!(definition["uri"], server.uri("main.zr"));
    assert_eq!(
        definition["range"]["start"],
        json!({ "line": 6, "character": 7 })
    );
    let definition = server.at("textDocument/definition", 20, 17);
    assert_eq!(definition["uri"], server.uri("util.zr"));

    let members = server.at("textDocument/completion", 20, 25);
    assert_eq!(labels(&members), ["x", "y", "sum"]);
    assert_eq!(members["items"][2]["insertText"], "sum()$0");
    let half_path = server.at("textDocument/completion", 20, 16);
    assert_eq!(half_path, json!({ "isIncomplete": true, "items": [] }));
    assert!(labels(&server.at("textDocument/completion", 20, 17)).contains(&"twice"));
    let functions = server.at("textDocument/completion", 18, 18);
    assert_eq!(labels(&functions), ["origin"]);
    assert!(labels(&server.at("textDocument/completion", 20, 6)).contains(&"push"));
    let names = server.at("textDocument/completion", 21, 4);
    assert!(
        ["p", "q", "v", "Point", "main", "util", "println", "while"]
            .iter()
            .all(|n| labels(&names).contains(n))
    );

    let help = server.at("textDocument/signatureHelp", 20, 23);
    assert_eq!(help["signatures"][0]["label"], "pub fn twice(n: i32) i32");
    assert_eq!(
        help["signatures"][0]["documentation"]["value"],
        "Twice `n`."
    );
    assert_eq!(
        server
            .at("textDocument/references", 3, 4)
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let renamed = server.at("textDocument/rename", 17, 8);
    assert_eq!(
        renamed["changes"][server.uri("main.zr")]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let symbols = server.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": server.uri("main.zr") } }),
    );
    let names = |symbols: &Value| -> Vec<String> {
        let symbols = symbols.as_array().unwrap().iter();
        symbols
            .map(|s| s["name"].as_str().unwrap().into())
            .collect()
    };
    assert_eq!(names(&symbols), ["Point", "main"]);
    assert_eq!(names(&symbols[0]["children"]), ["x", "y", "sum", "origin"]);

    let tokens = server.request(
        "textDocument/semanticTokens/full",
        json!({ "textDocument": { "uri": server.uri("main.zr") } }),
    );
    let data: Vec<u64> = tokens["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap())
        .collect();
    let mut classed = Vec::new();
    let (mut line, mut column) = (0, 0);
    for token in data.chunks(5) {
        line += token[0];
        column = if token[0] == 0 {
            column + token[1]
        } else {
            token[1]
        };
        classed.push((
            (line, column),
            legend[token[3] as usize].as_str().unwrap().to_string(),
        ));
    }
    let class = |line, column| {
        let found = classed.iter().find(|(at, _)| *at == (line, column));
        found.map(|(_, kind)| kind.as_str()).unwrap_or("none")
    };
    let expected = [
        ((0, 0), "keyword"),
        ((0, 7), "namespace"),
        ((2, 7), "struct"),
        ((3, 4), "property"),
        ((3, 7), "type"),
        ((6, 7), "method"),
        ((7, 15), "parameter"),
        ((15, 0), "comment"),
        ((17, 8), "variable"),
        ((17, 23), "number"),
        ((18, 18), "method"),
        ((20, 11), "namespace"),
        ((20, 17), "function"),
        ((21, 4), "macro"),
        ((21, 12), "string"),
    ];
    for ((line, column), kind) in expected {
        assert_eq!(class(line, column), kind, "at {line}:{column}");
    }

    let typed = MAIN.replace(
        "    var p = Point",
        "    var q2 = Point.origin();\n    q2.\n    var p = Point",
    );
    server.change(&typed);
    let live = server.diagnostics("main.zr");
    assert_eq!(live[0]["message"], "Expected a field name after '.'");
    assert_eq!(
        labels(&server.at("textDocument/completion", 18, 7)),
        ["x", "y", "sum"]
    );

    let broken = MAIN.replace("var q = Point.origin();", "var q: bool = 1;");
    std::fs::write(server.dir.join("main.zr"), &broken).unwrap();
    server.change(&broken);
    assert_eq!(server.diagnostics("main.zr"), Vec::<Value>::new());
    server.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": server.uri("main.zr") } }),
    );
    let sent = Instant::now();
    let hover = server.ask(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": server.uri("main.zr") },
            "position": { "line": 20, "character": 23 },
        }),
    );
    let mut answered = None;
    let checked = loop {
        let message = server.receive();
        if message["id"] == hover {
            answered = Some(sent.elapsed());
        }
        let diagnostics = &message["params"]["diagnostics"];
        if message["method"] == "textDocument/publishDiagnostics"
            && diagnostics.as_array().is_some_and(|d| !d.is_empty())
        {
            break diagnostics[0].clone();
        }
    };
    let answered = answered.expect("the hover is answered before the check ends");
    eprintln!(
        "hover during a check: {answered:?}, check: {:?}",
        sent.elapsed()
    );
    assert!(answered < Duration::from_millis(500));
    assert!(
        checked["message"]
            .as_str()
            .unwrap()
            .contains("Annotated as bool"),
        "{checked}"
    );
    assert_eq!(
        checked["range"]["start"],
        json!({ "line": 18, "character": 4 })
    );

    std::fs::write(server.dir.join("main.zr"), MAIN).unwrap();
    std::fs::write(server.dir.join("util.zr"), UTIL.replace("n * 2", "true")).unwrap();
    server.change(MAIN);
    server.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": server.uri("main.zr") } }),
    );
    let imported = server.diagnostics("util.zr");
    assert!(
        imported[0]["message"]
            .as_str()
            .unwrap()
            .contains("expects i32"),
        "{imported:?}"
    );
}
