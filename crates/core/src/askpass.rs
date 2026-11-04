//! Credential prompts from git/ssh, answered by the GlideGit UI.
//!
//! git and ssh ask for usernames, passwords, key passphrases and host-key
//! confirmations by running the program named in `GIT_ASKPASS` / `SSH_ASKPASS`
//! with the prompt as its only argument, and reading the answer from its stdout.
//!
//! GlideGit points both at its own executable. Started that way, the binary
//! runs [`client_main_if_requested`]: it connects to the [`AskpassServer`]
//! inside the running app over loopback TCP, authenticates with a random
//! per-launch token, sends the prompt and prints the answer the user typed.
//! Nothing is stored; answers only pass through memory.

use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use ts_rs::TS;

pub const ENV_ADDR: &str = "GLIDEGIT_ASKPASS_ADDR";
pub const ENV_TOKEN: &str = "GLIDEGIT_ASKPASS_TOKEN";

/// How long a prompt may wait for the user before git is told "no answer".
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AskKind {
    Username,
    Password,
    /// SSH private key passphrase.
    Passphrase,
    /// Yes/no question, e.g. accepting an unknown SSH host key.
    Confirm,
    Text,
}

/// A prompt shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AskRequest {
    pub id: u64,
    pub kind: AskKind,
    /// The raw prompt from git/ssh (may span several lines for host keys).
    pub prompt: String,
    /// URL, host or key file the prompt is about, when it can be extracted.
    pub target: Option<String>,
}

/// Classify a git/ssh prompt and pull out what it refers to.
pub fn classify(prompt: &str) -> (AskKind, Option<String>) {
    let quoted = || {
        let start = prompt.find('\'')? + 1;
        let end = prompt[start..].find('\'')? + start;
        Some(prompt[start..end].to_string())
    };
    let lower = prompt.to_ascii_lowercase();
    if lower.starts_with("username for") {
        (AskKind::Username, quoted())
    } else if lower.starts_with("password for") {
        (AskKind::Password, quoted())
    } else if lower.contains("passphrase") {
        (AskKind::Passphrase, quoted())
    } else if lower.contains("(yes/no") {
        let host = prompt
            .find("authenticity of host '")
            .map(|i| i + "authenticity of host '".len())
            .and_then(|s| prompt[s..].find('\'').map(|e| prompt[s..s + e].to_string()));
        (AskKind::Confirm, host)
    } else if lower.contains("password") || lower.contains("pin") {
        (AskKind::Password, quoted())
    } else {
        (AskKind::Text, quoted())
    }
}

#[derive(Serialize, Deserialize)]
struct WireRequest {
    token: String,
    prompt: String,
}

#[derive(Serialize, Deserialize)]
struct WireResponse {
    answer: Option<String>,
}

pub type AskFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;
/// Called for each prompt; resolves to the answer, or `None` if the user cancelled.
pub type AskHandler = Arc<dyn Fn(String) -> AskFuture + Send + Sync>;

/// Loopback listener plus the secret token that clients must present.
pub struct AskpassServer {
    listener: std::net::TcpListener,
    addr: SocketAddr,
    token: String,
}

impl AskpassServer {
    /// Bind synchronously (before the async runtime exists) so the address is
    /// known when the git CLI wrapper is configured.
    pub fn bind() -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            listener,
            addr,
            token,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Environment for git processes. `exe` is the program git should run
    /// (the GlideGit binary itself).
    pub fn git_env(&self, exe: &Path) -> Vec<(String, String)> {
        let exe = exe.to_string_lossy().into_owned();
        vec![
            ("GIT_ASKPASS".into(), exe.clone()),
            ("SSH_ASKPASS".into(), exe),
            // OpenSSH ≥ 8.4: use askpass even when a terminal is attached
            // (e.g. `tauri dev`), instead of blocking on a tty nobody sees.
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            (ENV_ADDR.into(), self.addr.to_string()),
            (ENV_TOKEN.into(), self.token.clone()),
        ]
    }

    /// Accept connections forever. Must run inside a tokio runtime.
    pub async fn serve(self, handler: AskHandler) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        let token: Arc<str> = self.token.into();
        loop {
            let (stream, _) = listener.accept().await?;
            let handler = handler.clone();
            let token = token.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, &token, handler).await {
                    tracing::warn!("askpass connection failed: {e}");
                }
            });
        }
    }
}

async fn handle_conn(
    stream: tokio::net::TcpStream,
    token: &str,
    handler: AskHandler,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    TokioBufReader::new(read)
        .take(64 * 1024)
        .read_line(&mut line)
        .await?;
    let req: WireRequest = serde_json::from_str(&line).map_err(std::io::Error::other)?;
    let answer = if constant_time_eq(req.token.as_bytes(), token.as_bytes()) {
        tokio::time::timeout(ANSWER_TIMEOUT, handler(req.prompt))
            .await
            .unwrap_or(None)
    } else {
        tracing::warn!("askpass request with invalid token rejected");
        None
    };
    let mut out = serde_json::to_vec(&WireResponse { answer }).map_err(std::io::Error::other)?;
    out.push(b'\n');
    write.write_all(&out).await?;
    write.shutdown().await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Ask the running app. Blocking; used by the askpass client process.
pub fn ask(addr: &str, token: &str, prompt: &str) -> std::io::Result<Option<String>> {
    let addr: SocketAddr = addr.parse().map_err(std::io::Error::other)?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    let mut msg = serde_json::to_vec(&WireRequest {
        token: token.to_string(),
        prompt: prompt.to_string(),
    })
    .map_err(std::io::Error::other)?;
    msg.push(b'\n');
    stream.write_all(&msg)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let resp: WireResponse = serde_json::from_str(&line).map_err(std::io::Error::other)?;
    Ok(resp.answer)
}

/// Entry point for the askpass client mode. Returns `Some(exit_code)` when this
/// process was started by git/ssh as an askpass helper, `None` otherwise.
pub fn client_main_if_requested() -> Option<i32> {
    let addr = std::env::var(ENV_ADDR).ok()?;
    // Only act as a helper when invoked with a prompt argument, so a git hook
    // that happens to launch the app normally is not misinterpreted.
    let prompt = std::env::args().nth(1)?;
    let token = std::env::var(ENV_TOKEN).unwrap_or_default();
    match ask(&addr, &token, &prompt) {
        Ok(Some(answer)) => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{answer}");
            let _ = out.flush();
            Some(0)
        }
        Ok(None) => Some(1),
        Err(e) => {
            eprintln!("glidegit askpass: {e}");
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_prompts() {
        assert_eq!(
            classify("Username for 'https://github.com': "),
            (AskKind::Username, Some("https://github.com".into()))
        );
        assert_eq!(
            classify("Password for 'https://min@github.com': "),
            (AskKind::Password, Some("https://min@github.com".into()))
        );
        assert_eq!(
            classify("Enter passphrase for key '/Users/min/.ssh/id_ed25519': "),
            (
                AskKind::Passphrase,
                Some("/Users/min/.ssh/id_ed25519".into())
            )
        );
        let host = "The authenticity of host 'github.com (140.82.112.3)' can't be established.\n\
ED25519 key fingerprint is SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU.\n\
Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert_eq!(
            classify(host),
            (AskKind::Confirm, Some("github.com (140.82.112.3)".into()))
        );
        assert_eq!(classify("Something else?").0, AskKind::Text);
    }

    #[test]
    fn constant_time() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
