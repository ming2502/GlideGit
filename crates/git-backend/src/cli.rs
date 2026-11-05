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
use crate::model::Progress;
use crate::parse::parse_progress_line;

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

/// Receives parsed `--progress` updates from a running git process.
#[derive(Clone)]
pub struct ProgressSink(Arc<dyn Fn(&Progress) + Send + Sync>);

impl ProgressSink {
    pub fn new(f: impl Fn(&Progress) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }
    pub fn emit(&self, p: &Progress) {
        (self.0)(p)
    }
}

impl std::fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressSink")
    }
}

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub stdin: Option<Vec<u8>>,
    pub cancel: Option<CancelToken>,
    pub timeout: Option<Duration>,
    /// Read-only commands set `GIT_OPTIONAL_LOCKS=0` so they never contend with writers.
    pub read_only: bool,
    /// Parse stderr progress lines (`--progress`) and forward them here.
    /// Progress lines are then left out of the captured stderr.
    pub progress: Option<ProgressSink>,
    /// Extra environment for this command; overrides the defaults.
    pub env: Vec<(String, String)>,
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
    pub fn progress(mut self, sink: ProgressSink) -> Self {
        self.progress = Some(sink);
        self
    }
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
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
        Self::discover_with(&[])
    }

    /// Like [`GitCli::discover`], trying `fallbacks` last: a git shipped
    /// with the app (MinGit on Windows) is only used when the user has none,
    /// so their own git, its version and its configuration take precedence.
    pub fn discover_with(fallbacks: &[PathBuf]) -> Result<Self> {
        if let Some(exe) = std::env::var_os("GLIDEGIT_GIT") {
            return Self::with_executable(exe);
        }
        let mut last_err = GitError::GitNotFound("git".into());
        for c in candidates(fallbacks) {
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

    fn command(&self, cwd: &Path, args: &[&str], opts: &RunOptions) -> Command {
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
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Own process group, so cancelling also stops helpers git spawned
        // (ssh, git-remote-https, hooks) instead of orphaning them.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        cmd
    }

    /// Run git and return the output whatever the exit code.
    pub fn run_unchecked(&self, cwd: &Path, args: &[&str], opts: RunOptions) -> Result<GitOutput> {
        let mut cmd = self.command(cwd, args, &opts);
        let mut child = spawn(&mut cmd)?;

        // Feed stdin and drain stdout/stderr on helper threads to avoid pipe deadlocks.
        let stdin_thread = match (opts.stdin, child.stdin.take()) {
            (Some(data), Some(mut pipe)) => Some(thread::spawn(move || {
                let _ = pipe.write_all(&data);
            })),
            _ => None,
        };
        let stdout_thread = child.stdout.take().map(spawn_reader);
        let stderr_thread = match opts.progress {
            Some(sink) => child.stderr.take().map(|r| spawn_progress_reader(r, sink)),
            None => child.stderr.take().map(spawn_reader),
        };

        let status = if opts.cancel.is_none() && opts.timeout.is_none() {
            child.wait()?
        } else {
            let started = Instant::now();
            loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if opts.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                    kill_tree(&mut child);
                    return Err(GitError::Cancelled);
                }
                if opts.timeout.is_some_and(|t| started.elapsed() > t) {
                    kill_tree(&mut child);
                    return Err(GitError::TimedOut);
                }
                thread::sleep(Duration::from_millis(4));
            }
        };

        if let Some(t) = stdin_thread {
            let _ = t.join();
        }
        let stdout = stdout_thread.map(finish_reader).unwrap_or_default();
        let stderr = stderr_thread.map(finish_reader).unwrap_or_default();

        Ok(GitOutput {
            stdout,
            stderr,
            code: status.code(),
        })
    }
}

impl GitCli {
    /// Run git and hand its stdout to `on_stdout` chunk by chunk as it arrives,
    /// for output too large to buffer (`git log` over a whole repository).
    ///
    /// Returning `false` from the callback stops git early; that is not an error.
    /// A non-zero exit after the output was fully read is reported as
    /// [`GitError::Failed`].
    pub fn run_streaming(
        &self,
        cwd: &Path,
        args: &[&str],
        opts: RunOptions,
        mut on_stdout: impl FnMut(&[u8]) -> bool,
    ) -> Result<()> {
        let mut cmd = self.command(cwd, args, &opts);
        let mut child = spawn(&mut cmd)?;
        let stdin_thread = match (opts.stdin, child.stdin.take()) {
            (Some(data), Some(mut pipe)) => Some(thread::spawn(move || {
                let _ = pipe.write_all(&data);
            })),
            _ => None,
        };
        let stderr_thread = child.stderr.take().map(spawn_reader);
        let mut stdout = child.stdout.take().expect("stdout is piped");

        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if opts.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                kill_tree(&mut child);
                return Err(GitError::Cancelled);
            }
            let n = match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    kill_tree(&mut child);
                    return Err(e.into());
                }
            };
            if !on_stdout(&buf[..n]) {
                kill_tree(&mut child);
                return Ok(());
            }
        }
        let status = child.wait()?;
        if let Some(t) = stdin_thread {
            let _ = t.join();
        }
        let stderr = stderr_thread.map(finish_reader).unwrap_or_default();
        if status.success() {
            Ok(())
        } else {
            Err(GitError::Failed {
                command: format!("git {}", args.join(" ")),
                code: status.code(),
                message: clean_stderr(&String::from_utf8_lossy(&stderr)),
            })
        }
    }
}

/// Kill git and, on Unix, every process in its group. Reader threads are
/// detached: a surviving grandchild could otherwise keep the pipes open.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: plain syscall; the group id is the child's pid (process_group(0)).
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// How long to wait for git's output pipes to close once git has exited.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// The output a reader thread collected. Normally the pipe closes when git
/// exits; if another process still holds it (a grandchild git left running,
/// or a pipe inherited by a process spawned at the same moment), give up
/// after [`PIPE_GRACE`] rather than wait for that process.
fn finish_reader(t: thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    let deadline = Instant::now() + PIPE_GRACE;
    while !t.is_finished() {
        if Instant::now() > deadline {
            return Vec::new();
        }
        thread::sleep(Duration::from_millis(1));
    }
    t.join().unwrap_or_default()
}

fn spawn_reader<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

/// Split stderr on `\r`/`\n` as it streams in; progress lines go to the sink,
/// everything else is kept as the captured stderr.
fn spawn_progress_reader<R: Read + Send + 'static>(
    mut r: R,
    sink: ProgressSink,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut line = Vec::new();
        let mut buf = [0u8; 4096];
        let handle = |line: &mut Vec<u8>, kept: &mut Vec<u8>| {
            if line.is_empty() {
                return;
            }
            let text = String::from_utf8_lossy(line);
            match parse_progress_line(&text) {
                Some(p) => sink.emit(&p),
                None => {
                    kept.extend_from_slice(line);
                    kept.push(b'\n');
                }
            }
            line.clear();
        };
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                if b == b'\r' || b == b'\n' {
                    handle(&mut line, &mut kept);
                } else {
                    line.push(b);
                }
            }
        }
        handle(&mut line, &mut kept);
        kept
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

/// Where to look for git, in order. Existing fallbacks come last.
fn candidates(fallbacks: &[PathBuf]) -> Vec<PathBuf> {
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
    candidates.extend(fallbacks.iter().filter(|p| p.is_file()).cloned());
    candidates
}

/// Start a child process, one at a time.
///
/// On macOS the standard library creates a child's pipes and only then marks
/// them close-on-exec. A process started from another thread in between
/// inherits them, and a short command's output then stays open until that
/// other (possibly long-running) process exits: a quick `git status` could
/// wait for a whole `git log` of a million commits, or deadlock with it.
/// Spawning takes a millisecond or two; serialising it costs nothing.
fn spawn(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    static SPAWN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SPAWN.lock().unwrap_or_else(|e| e.into_inner());
    cmd.spawn()
}

#[cfg(test)]
mod tests {

    #[test]
    fn bundled_git_is_the_last_resort() {
        let dir = std::env::temp_dir().join(format!("glidegit-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let present = dir.join("git-present");
        std::fs::write(&present, "").unwrap();
        let missing = dir.join("git-missing");
        let c = candidates(&[missing, present.clone()]);
        // PATH's git comes before it; a fallback that does not exist is skipped.
        assert_eq!(c.last(), Some(&present));
        assert!(
            c.iter().position(|p| p == Path::new("git")) < c.iter().position(|p| *p == present)
        );
        assert_eq!(c.iter().filter(|p| p.starts_with(&dir)).count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
