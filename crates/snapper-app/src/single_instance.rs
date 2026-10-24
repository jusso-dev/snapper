//! One Snapper process per user.
//!
//! The primary listens on a local socket named `snapper-<username>`. A second
//! launch connects, sends its CLI arguments as one JSON line, and exits 0.
//! The primary reads that line from [`serve`].

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{
    prelude::*, GenericNamespaced, Listener, ListenerOptions, Name, Stream,
};
use serde::{Deserialize, Serialize};

/// Result of trying to become the resident process.
pub enum Claim {
    /// This process owns the socket.
    Primary(Listener),
    /// Arguments were sent to the process that already owns it.
    Forwarded,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ForwardedArgs {
    args: Vec<String>,
}

/// Bind `snapper-<username>`, or forward `args` when another process has it.
pub fn claim(args: &[String]) -> Result<Claim, String> {
    let socket_name = socket_name_for(&current_username());
    bind_or_forward(&socket_name, args)
}

fn bind_or_forward(socket_name: &str, args: &[String]) -> Result<Claim, String> {
    let name = namespaced(socket_name)?;
    match ListenerOptions::new().name(name).create_sync() {
        Ok(listener) => Ok(Claim::Primary(listener)),
        Err(err) if socket_is_taken(&err) => match forward(socket_name, args) {
            Ok(()) => Ok(Claim::Forwarded),
            // A socket file left by a process that was killed refuses
            // connections. Nobody owns it, so replace it.
            Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
                ListenerOptions::new()
                    .name(namespaced(socket_name)?)
                    .try_overwrite(true)
                    .create_sync()
                    .map(Claim::Primary)
                    .map_err(|err| err.to_string())
            }
            Err(err) => Err(err.to_string()),
        },
        Err(err) => Err(err.to_string()),
    }
}

/// `AddrInUse` on Unix. On Windows a second `CreateNamedPipe` with
/// `FILE_FLAG_FIRST_PIPE_INSTANCE` fails with `ERROR_ACCESS_DENIED` (5)
/// when the name is already owned.
fn socket_is_taken(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::AddrInUse {
        return true;
    }
    cfg!(windows) && err.raw_os_error() == Some(5)
}

/// Block on `listener` and send each JSON argument list to `tx`.
pub fn serve(listener: Listener, tx: std::sync::mpsc::Sender<Vec<String>>) {
    loop {
        match listener.accept() {
            Ok(stream) => {
                if tx.send(read_forward(stream)).is_err() {
                    break;
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "single-instance listener failed");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}

fn read_forward(stream: Stream) -> Vec<String> {
    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    match reader.read_line(&mut line) {
        Ok(_) => decode_args(&line).unwrap_or_default(),
        Err(err) => {
            tracing::warn!(error = %err, "could not read forwarded arguments");
            Vec::new()
        }
    }
}

fn forward(socket_name: &str, args: &[String]) -> std::io::Result<()> {
    let name = namespaced(socket_name).map_err(std::io::Error::other)?;
    let mut stream = Stream::connect(name)?;
    let mut line = encode_args(args);
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

fn namespaced(socket_name: &str) -> Result<Name<'_>, String> {
    socket_name
        .to_ns_name::<GenericNamespaced>()
        .map_err(|err| err.to_string())
}

/// `snapper-` plus a filesystem-safe username.
pub fn socket_name_for(username: &str) -> String {
    let mut safe = String::new();
    for ch in username.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            safe.push(ch);
        } else {
            safe.push('_');
        }
    }
    if safe.is_empty() || safe.chars().all(|ch| ch == '_') {
        safe = "user".to_owned();
    }
    const MAX: usize = 64;
    if safe.len() > MAX {
        safe.truncate(MAX);
    }
    format!("snapper-{safe}")
}

fn current_username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "user".to_owned())
}

pub fn encode_args(args: &[String]) -> String {
    serde_json::to_string(&ForwardedArgs {
        args: args.to_vec(),
    })
    .unwrap_or_else(|_| "{\"args\":[]}".to_owned())
}

pub fn decode_args(line: &str) -> Option<Vec<String>> {
    let parsed: ForwardedArgs = serde_json::from_str(line.trim()).ok()?;
    Some(parsed.args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_forwards_its_arguments() {
        let name = format!("snapper-test-{}", std::process::id());
        let Claim::Primary(listener) = bind_or_forward(&name, &[]).expect("primary") else {
            panic!("the first claim should own the socket");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            tx.send(read_forward(stream)).expect("send");
        });
        let args = vec!["--open-editor".to_owned(), "shot.png".to_owned()];
        let Claim::Forwarded = bind_or_forward(&name, &args).expect("forward") else {
            panic!("the second claim should forward");
        };
        let got = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("arguments");
        server.join().expect("server");
        assert_eq!(got, args);
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_left_by_a_killed_process_is_replaced() {
        let name = format!("snapper-test-stale-{}", std::process::id());
        let stale = ListenerOptions::new()
            .name(namespaced(&name).expect("name"))
            .reclaim_name(false)
            .create_sync()
            .expect("bind");
        drop(stale);
        let Claim::Primary(_listener) = bind_or_forward(&name, &[]).expect("claim") else {
            panic!("the stale socket should be replaced");
        };
    }

    #[test]
    fn socket_name_keeps_letters_and_replaces_the_rest() {
        assert_eq!(socket_name_for("agent"), "snapper-agent");
        assert_eq!(socket_name_for("DOMAIN\\agent"), "snapper-DOMAIN_agent");
        assert_eq!(socket_name_for("a b"), "snapper-a_b");
        assert_eq!(socket_name_for(""), "snapper-user");
        assert_eq!(socket_name_for("***"), "snapper-user");
        let long = "u".repeat(80);
        assert_eq!(socket_name_for(&long).len(), "snapper-".len() + 64);
    }

    #[test]
    fn arguments_round_trip_as_json() {
        let args = vec!["--open-editor".to_owned(), "shot.png".to_owned()];
        let line = encode_args(&args);
        assert!(line.starts_with('{'), "{line}");
        assert_eq!(decode_args(&line), Some(args));
        assert_eq!(decode_args("{\"args\":[]}\n"), Some(Vec::<String>::new()));
        assert_eq!(decode_args("not json"), None);
    }
}
