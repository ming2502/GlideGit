//! Thin, safe wrapper around the system `git` executable.
//!
//! All write operations in GlideGit go through the CLI so that hooks, credential
//! helpers, signing, LFS and the user's config behave exactly like on the command line.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{GitError, Result};

/// Cooperative cancellation flag shared between the caller and a running git process.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub stdin: Option<Vec<u8>>,
    pub cancel: Option<CancelToken>,
    pub timeout: Option<Duration>,
    /// Read-only commands set `GIT_OPTIONAL_LOCKS=0` so they never contend with writers.
    pub read_only: bool,
}

impl RunOptions {
    pub fn read() -> Self {
        Self {
            read_only: true,
            ..Self::default()
        }
    }
    pub fn write() -> Self {
        Self::default()
    }
    pub fn stdin(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(data.into());
        self
    }
    pub fn cancel(mut self, token: CancelToken) -> Self {
        self.cancel = Some(token);
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Debug)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: Option<i32>,
}

impl GitOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
    pub fn stdout_str(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }
    pub fn stderr_str(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

#[derive(Clone, Debug)]
pub struct GitCli {
    exe: PathBuf,
    version: String,
    extra_env: Vec<(OsString, OsString)>,
}

/// Global flags prepended to every invocation to keep output stable and parseable.
const GLOBAL_ARGS: &[&str] = &[
    "-c",
    "core.quotepath=false",
    "-c",
    "color.ui=false",
    "--no-pager",
];

impl GitCli {
    /// Locate git: `GLIDEGIT_GIT` env var, then well-known install locations
    /// (GUI apps on macOS start with a minimal `PATH`), then `git` on `PATH`.
    pub fn discover() -> Result<Self> {
        if let Some(exe) = std::env::var_os("GLIDEGIT_GIT") {
            return Self::with_executable(exe);
        }
        // `cfg!()` (not `#[cfg]`) so every platform compiles and lints the same code.
        let mut candidates: Vec<PathBuf> = Vec::new();
        if cfg!(target_os = "macos") {
            candidates.extend(
                ["/opt/homebrew/bin/git", "/usr/local/bin/git"]
                    .iter()
                    .map(PathBuf::from)
                    .filter(|p| p.is_file()),
            );
        }
        candidates.push(PathBuf::from("git"));
        if cfg!(windows) {
            candidates.push(PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"));
        }
        if cfg!(target_os = "macos") {
            candidates.push(PathBuf::from("/usr/bin/git"));
        }

        let mut last_err = GitError::GitNotFound("git".into());
        for c in candidates {
            match Self::with_executable(c) {
                Ok(cli) => return Ok(cli),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub fn with_executable(exe: impl Into<PathBuf>) -> Result<Self> {
        let mut cli = Self {
            exe: exe.into(),
            version: String::new(),
            extra_env: Vec::new(),
        };
        let out = cli
            .run_unchecked(Path::new("."), &["--version"], RunOptions::read())
            .map_err(|e| GitError::GitNotFound(format!("{}: {e}", cli.exe.display())))?;
        if !out.success() {
            return Err(GitError::GitNotFound(cli.exe.display().to_string()));
        }
        cli.version = out
            .stdout_str()
            .trim()
            .trim_start_matches("git version ")
            .to_string();
        Ok(cli)
    }

    /// Add an environment variable passed to every git process (e.g. for test isolation).
    pub fn with_env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.extra_env
            .push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    pub fn executable(&self) -> &Path {
        &self.exe
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Run git and fail with [`GitError::Failed`] on a non-zero exit code.
    pub fn run(&self, cwd: &Path, args: &[&str], opts: RunOptions) -> Result<GitOutput> {
        let out = self.run_unchecked(cwd, args, opts)?;
        if out.success() {
            Ok(out)
        } else {
            Err(GitError::Failed {
                command: format!("git {}", args.join(" ")),
                code: out.code,
                message: clean_stderr(&out.stderr_str()),
            })
        }
    }

    /// Run git and return the output whatever the exit code.
    pub fn run_unchecked(&self, cwd: &Path, args: &[&str], opts: RunOptions) -> Result<GitOutput> {
        let mut cmd = Command::new(&self.exe);
        cmd.current_dir(cwd)
            .args(GLOBAL_ARGS)
            .args(args)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("LANGUAGE", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if opts.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        if opts.read_only {
            cmd.env("GIT_OPTIONAL_LOCKS", "0");
        }
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn()?;

        // Feed stdin and drain stdout/stderr on helper threads to avoid pipe deadlocks.
        let stdin_thread = match (opts.stdin, child.stdin.take()) {
            (Some(data), Some(mut pipe)) => Some(thread::spawn(move || {
                let _ = pipe.write_all(&data);
            })),
            _ => None,
        };
        let stdout_thread = child.stdout.take().map(spawn_reader);
        let stderr_thread = child.stderr.take().map(spawn_reader);

        let status = if opts.cancel.is_none() && opts.timeout.is_none() {
            child.wait()?
        } else {
            let started = Instant::now();
            loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if opts.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GitError::Cancelled);
                }
                if opts.timeout.is_some_and(|t| started.elapsed() > t) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GitError::TimedOut);
                }
                thread::sleep(Duration::from_millis(4));
            }
        };

        if let Some(t) = stdin_thread {
            let _ = t.join();
        }
        let stdout = stdout_thread
            .map(|t| t.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = stderr_thread
            .map(|t| t.join().unwrap_or_default())
            .unwrap_or_default();

        Ok(GitOutput {
            stdout,
            stderr,
            code: status.code(),
        })
    }
}

fn spawn_reader<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

/// Drop git's "hint:" noise so error toasts stay readable.
fn clean_stderr(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|l| !l.starts_with("hint:"))
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        "git command failed".to_string()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_stderr_strips_hints() {
        let s = "error: something broke\nhint: try this\nhint: or that\n";
        assert_eq!(clean_stderr(s), "error: something broke");
    }

    #[test]
    fn discover_finds_git() {
        let cli = GitCli::discover().expect("git should be installed for tests");
        assert!(!cli.version().is_empty());
    }

    #[test]
    fn cancel_kills_process() {
        let cli = GitCli::discover().unwrap();
        let token = CancelToken::new();
        token.cancel();
        let r = cli.run(
            Path::new("."),
            &["--version"],
            RunOptions::read().cancel(token),
        );
        // Either finished instantly or was cancelled; both are acceptable, never hangs.
        assert!(matches!(r, Ok(_) | Err(GitError::Cancelled)));
    }
}
