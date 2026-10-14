//! Example upload plugin.
//!
//! `snapper-plugin-echo --describe` prints a JSON Schema.
//! A request on stdin returns `{"ok":true,"url":"file://..."}`.
//! `config.mode` of `bad` prints a line that is not JSON.
//! `config.mode` of `sleep` waits, so the host can test its timeout.
//! `config.mode` of `fail` returns `{"ok":false,"error":"..."}`.

use std::io::{self, Read};
use std::thread;
use std::time::Duration;

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--describe") {
        println!(
            "{}",
            serde_json::json!({
                "title": "Echo",
                "type": "object",
                "properties": {
                    "note": { "type": "string", "title": "Note" }
                }
            })
        );
        return;
    }

    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        println!(r#"{{"ok":false,"error":"could not read stdin"}}"#);
        return;
    }
    let value: serde_json::Value = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(err) => {
            println!(
                "{}",
                serde_json::json!({"ok": false, "error": format!("bad request: {err}")})
            );
            return;
        }
    };
    match value.pointer("/config/mode").and_then(|item| item.as_str()) {
        Some("bad") => {
            println!("this is not json");
        }
        Some("sleep") => thread::sleep(Duration::from_secs(30)),
        Some("fail") => {
            println!(r#"{{"ok":false,"error":"plugin refused"}}"#);
        }
        _ => {
            let file = value
                .get("file")
                .and_then(|item| item.as_str())
                .unwrap_or("");
            println!(
                "{}",
                serde_json::json!({"ok": true, "url": format!("file://{file}")})
            );
        }
    }
}
