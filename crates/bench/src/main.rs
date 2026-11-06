//! Performance benchmarks, checked against the targets in docs/DESIGN.md §1.2.
//!
//! ```text
//! cargo run --release -p glide-bench -- [--commits N] [--files N] [--refs N]
//!     [--work DIR] [--slack F] [--json FILE] [--markdown FILE] [--no-fail]
//!     [--generate-only] [--only log,open,branches,diff,save]
//! ```
//!
//! Generates (or reuses, under `--work`) a synthetic repository, then times
//! what the UI waits for. Each measured value is compared with its target
//! times `--slack` (shared CI runners are slower and noisier than a desktop);
//! the exit status is 1 when any check fails, unless `--no-fail`.
//!
//! The rendering targets (Log scrolling at 60 fps, opening the branch popup)
//! depend on the WebView, not on this code. What the backend contributes is
//! checked instead: reading a page of the Log must fit in a frame; the popup
//! shows the branch list already in the snapshot, so the check is how fast
//! that list is refreshed after refs change (one git process, 100 ms budget:
//! starting a process alone can take 50–100 ms on a Windows CI runner).

mod generate;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use generate::{changed_path, Shape};
use glide_core::RepositoryManager;
use glide_git_backend::{CancelToken, GitCli, Repo};
use glide_log_index::LogIndex;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct Check {
    id: &'static str,
    /// What the user waits for (matches the table in DESIGN.md).
    label: &'static str,
    target_ms: f64,
    value_ms: f64,
    pass: bool,
    /// Extra detail: sample count, what was measured.
    note: String,
}

#[derive(Debug, Serialize)]
struct Report {
    os: &'static str,
    shape: Shape,
    slack: f64,
    generated_in_ms: Option<f64>,
    checks: Vec<Check>,
    /// Numbers worth watching that have no target.
    info: Vec<(String, f64)>,
}

struct Args {
    shape: Shape,
    work: PathBuf,
    slack: f64,
    json: Option<PathBuf>,
    markdown: Option<PathBuf>,
    fail: bool,
    generate_only: bool,
    /// Run only these groups (log, open, branches, diff, save).
    only: Option<Vec<String>>,
}

fn parse_args() -> Args {
    let mut a = Args {
        shape: Shape {
            commits: 1_000_000,
            files: 20_000,
            refs: 800,
        },
        work: std::env::temp_dir().join("glidegit-bench"),
        slack: 1.0,
        json: None,
        markdown: None,
        fail: true,
        generate_only: false,
        only: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--commits" => a.shape.commits = val().parse().expect("--commits N"),
            "--files" => a.shape.files = val().parse().expect("--files N"),
            "--refs" => a.shape.refs = val().parse().expect("--refs N"),
            "--work" => a.work = val().into(),
            "--slack" => a.slack = val().parse().expect("--slack F"),
            "--json" => a.json = Some(val().into()),
            "--markdown" => a.markdown = Some(val().into()),
            "--no-fail" => a.fail = false,
            "--generate-only" => a.generate_only = true,
            "--only" => a.only = Some(val().split(',').map(str::to_string).collect()),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn cli() -> GitCli {
    GitCli::discover().expect("git is installed")
}

struct Bench {
    checks: Vec<Check>,
    info: Vec<(String, f64)>,
    slack: f64,
}

impl Bench {
    fn check(
        &mut self,
        id: &'static str,
        label: &'static str,
        target_ms: f64,
        value: Duration,
        note: String,
    ) {
        let value_ms = ms(value);
        let pass = value_ms <= target_ms * self.slack;
        println!(
            "{} {label}: {value_ms:.1} ms (target {target_ms} ms) {note}",
            if pass { "PASS" } else { "FAIL" }
        );
        self.checks.push(Check {
            id,
            label,
            target_ms,
            value_ms,
            pass,
            note,
        });
    }
    fn info(&mut self, what: impl Into<String>, value: Duration) {
        let what = what.into();
        println!("     {what}: {:.1} ms", ms(value));
        self.info.push((what, ms(value)));
    }
}

/// Run `f` on a fresh runtime (each open uses its own manager and watcher).
fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

/// Open the repository and wait for its first snapshot. Returns how long
/// that took and whether the snapshot came from the cache.
fn open_once(repo: &Path, cache: &Path) -> (Duration, bool) {
    block_on(async {
        let manager = RepositoryManager::new(cli());
        manager.set_cache_dir(cache.to_path_buf());
        let t = Instant::now();
        let (session, _) = manager.open(repo).await.unwrap();
        let took = t.elapsed();
        let stale = session.snapshot().unwrap().stale;
        // Let the background work (fresh snapshot, cache write, log index)
        // finish so the next run starts from a settled state.
        if let Some(mut rx) = session.subscribe_log() {
            let _ = tokio::time::timeout(Duration::from_secs(600), async {
                while !rx.borrow_and_update().complete {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        manager.close(&session.root().to_string_lossy()).await;
        (took, stale)
    })
}

fn bench_open(b: &mut Bench, repo: &Path, scratch: &Path) {
    let cache = scratch.join("cache-open");
    let _ = std::fs::remove_dir_all(&cache);
    let (cold, stale) = open_once(repo, &cache);
    assert!(!stale);
    b.check(
        "cold_open",
        "冷启动到可交互（无快照）",
        800.0,
        cold,
        "打开仓库到第一个快照".into(),
    );
    let warm: Vec<Duration> = (0..5)
        .map(|_| {
            let (t, stale) = open_once(repo, &cache);
            assert!(
                stale,
                "the second open should start from the cached snapshot"
            );
            t
        })
        .collect();
    b.check(
        "warm_open",
        "冷启动到可交互（有快照）",
        300.0,
        median(warm),
        "5 次中位数".into(),
    );
}

fn bench_log(b: &mut Bench, repo_path: &Path, scratch: &Path) {
    let repo = Repo::open(cli(), repo_path).unwrap();
    // Without a commit-graph (what a first open of a fresh clone may see,
    // until the app writes one): for reference only.
    let no_graph = Repo::open(
        cli()
            .with_env("GIT_CONFIG_COUNT", "1")
            .with_env("GIT_CONFIG_KEY_0", "core.commitGraph")
            .with_env("GIT_CONFIG_VALUE_0", "false"),
        repo_path,
    )
    .unwrap();
    let dir = scratch.join("cache-log-nograph");
    let _ = std::fs::remove_dir_all(&dir);
    let (first, full, total) = build_index(&no_graph, &dir);
    b.info(
        format!("Log 首次建索引（无 commit-graph，{total} 个提交）首屏"),
        first,
    );
    b.info("Log 首次建索引（无 commit-graph）全量", full);

    // First rows of a fresh index, five times (each stopped once they are
    // in): one run on a shared CI machine can be several times slower.
    let firsts: Vec<Duration> = (0..5)
        .map(|i| first_rows(&repo, &scratch.join(format!("cache-log-first-{i}"))))
        .collect();
    let dir = scratch.join("cache-log");
    let _ = std::fs::remove_dir_all(&dir);
    let (_, full, total) = build_index(&repo, &dir);
    b.check(
        "log_first_page",
        "Log 首屏（首次打开）",
        500.0,
        median(firsts),
        format!(
            "{total} 个提交，有 commit-graph；5 次中位数；全量索引 {:.1} s",
            full.as_secs_f64()
        ),
    );

    // Reopening: rows are readable before the index has loaded.
    let times: Vec<Duration> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let index = LogIndex::open(&dir, repo.root()).unwrap();
            let page = index.page(0, 100).unwrap();
            assert_eq!(page.rows.len(), 100);
            t.elapsed()
        })
        .collect();
    b.check(
        "log_reopen_page",
        "Log 首屏（再次打开）",
        500.0,
        median(times),
        "打开索引并读首页 100 行，5 次中位数".into(),
    );

    // Scrolling: one page anywhere, worst case just before a checkpoint.
    let index = LogIndex::open(&dir, repo.root()).unwrap();
    index.update(&repo, &CancelToken::new(), &|_| {}).unwrap();
    let total = index.status().total;
    let mut times = Vec::new();
    for k in 0..20u32 {
        let start = ((total as u64 * k as u64 / 20) as u32 / 256) * 256 + 255;
        let start = start.min(total.saturating_sub(100));
        let t = Instant::now();
        let page = index.page(start, 100).unwrap();
        times.push(t.elapsed());
        assert!(!page.rows.is_empty());
    }
    let worst = *times.iter().max().unwrap();
    b.check(
        "log_scroll_page",
        "Log 滚动：读一页（一帧之内）",
        16.0,
        worst,
        "20 个位置中最慢的一次，含图布局".into(),
    );
}

/// Build the index from scratch: time to the first rows and to the end.
/// Time to the first rows of a fresh index; the build is cancelled there.
fn first_rows(repo: &Repo, dir: &Path) -> Duration {
    let _ = std::fs::remove_dir_all(dir);
    let index = LogIndex::open(dir, repo.root()).unwrap();
    let cancel = CancelToken::new();
    let started = Instant::now();
    let first = Mutex::new(None);
    let _ = index.update(repo, &cancel, &|s| {
        let mut f = first.lock().unwrap();
        if f.is_none() && s.total > 0 {
            *f = Some(started.elapsed());
            cancel.cancel();
        }
    });
    let took = first.into_inner().unwrap().expect("first rows arrive");
    drop(index);
    let _ = std::fs::remove_dir_all(dir);
    took
}

fn build_index(repo: &Repo, dir: &Path) -> (Duration, Duration, u32) {
    let index = LogIndex::open(dir, repo.root()).unwrap();
    let started = Instant::now();
    let first = Mutex::new(None);
    index
        .update(repo, &CancelToken::new(), &|s| {
            let mut f = first.lock().unwrap();
            if f.is_none() && s.total > 0 {
                *f = Some(started.elapsed());
            }
        })
        .unwrap();
    let full = started.elapsed();
    let first = first.into_inner().unwrap().unwrap_or(full);
    (first, full, index.status().total)
}

fn bench_branches(b: &mut Bench, repo_path: &Path) {
    let repo = Repo::open(cli(), repo_path).unwrap();
    let times: Vec<Duration> = (0..10)
        .map(|_| {
            let t = Instant::now();
            let list = repo.branches().unwrap();
            assert!(!list.local.is_empty());
            t.elapsed()
        })
        .collect();
    let n = repo.branches().unwrap().local.len();
    b.check(
        "branches",
        "引用变化后刷新分支列表",
        100.0,
        median(times),
        format!("{n} 个本地分支，10 次中位数；打开分支弹窗不调用后端，直接用快照中的列表"),
    );
}

fn bench_commit_diff(b: &mut Bench, repo_path: &Path) {
    let repo = Repo::open(cli(), repo_path).unwrap();
    let commits = repo.log(None, 200).unwrap();
    let mut times = Vec::new();
    for c in commits.iter().filter(|c| c.parents.len() == 1).take(10) {
        let t = Instant::now();
        let details = repo.commit_details(&c.oid).unwrap();
        let f = &details.files[0];
        let texts = repo
            .commit_file_texts(&c.oid, Some(&c.parents[0]), &f.path, f.orig_path.as_deref())
            .unwrap();
        assert!(texts.new.is_some());
        times.push(t.elapsed());
    }
    b.check(
        "commit_diff",
        "选中提交 → 显示 diff",
        100.0,
        median(times),
        "提交详情 + 第一个文件两侧全文，10 个提交的中位数".into(),
    );
}

/// Save a file and wait for the snapshot that shows it: watcher, debounce
/// and `git status` together.
fn bench_save(b: &mut Bench, repo: &Path, scratch: &Path, shape: Shape) {
    let rel = changed_path(shape);
    let file = repo.join(&rel);
    let original = std::fs::read(&file).unwrap();
    let times = block_on(async {
        let manager = RepositoryManager::new(cli());
        manager.set_cache_dir(scratch.join("cache-open"));
        let (session, _) = manager.open(repo).await.unwrap();
        // Wait for the start-up work so it does not compete.
        if let Some(mut rx) = session.subscribe_log() {
            while !rx.borrow_and_update().complete {
                rx.changed().await.unwrap();
            }
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        let mut rx = session.subscribe();
        let mut times = Vec::new();
        for round in 0..6 {
            let modified = round % 2 == 0;
            rx.borrow_and_update();
            let t = Instant::now();
            if modified {
                std::fs::write(&file, format!("edited in round {round}\n")).unwrap();
            } else {
                std::fs::write(&file, &original).unwrap();
            }
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    rx.changed().await.unwrap();
                    let has = rx
                        .borrow_and_update()
                        .as_ref()
                        .is_some_and(|s| s.changes.iter().any(|c| c.path == rel));
                    if has == modified {
                        break;
                    }
                }
            })
            .await
            .expect("the change shows up");
            times.push(t.elapsed());
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        manager.close(&session.root().to_string_lossy()).await;
        times
    });
    b.check(
        "save_to_panel",
        "保存文件 → Commit 面板更新",
        300.0,
        median(times),
        format!(
            "{} 个文件的工作区，含 40 ms 事件合并窗口和 git status；6 次中位数",
            shape.files
        ),
    );
}

fn main() {
    let args = parse_args();
    let repo = args.work.join(format!(
        "repo-{}c-{}f-{}r",
        args.shape.commits, args.shape.files, args.shape.refs
    ));
    let scratch = args.work.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();

    println!("repository: {} ({:?})", repo.display(), args.shape);
    let t = Instant::now();
    let generated = generate::ensure(&repo, args.shape);
    let generated_in_ms = generated.then(|| ms(t.elapsed()));
    if let Some(ms) = generated_in_ms {
        println!("generated in {:.1} s", ms / 1000.0);
    }
    if args.generate_only {
        return;
    }

    let mut b = Bench {
        checks: Vec::new(),
        info: Vec::new(),
        slack: args.slack,
    };
    let run = |group: &str| {
        args.only
            .as_ref()
            .is_none_or(|o| o.iter().any(|g| g == group))
    };
    // The Log group last: its full index builds keep the machine busy for
    // a while, which would show in whatever runs right after.
    if run("open") {
        bench_open(&mut b, &repo, &scratch);
    }
    if run("branches") {
        bench_branches(&mut b, &repo);
    }
    if run("diff") {
        bench_commit_diff(&mut b, &repo);
    }
    if run("save") {
        bench_save(&mut b, &repo, &scratch, args.shape);
    }
    if run("log") {
        bench_log(&mut b, &repo, &scratch);
    }

    let report = Report {
        os: std::env::consts::OS,
        shape: args.shape,
        slack: args.slack,
        generated_in_ms,
        checks: b.checks,
        info: b.info,
    };
    if let Some(p) = &args.json {
        std::fs::write(p, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    let md = markdown(&report);
    if let Some(p) = &args.markdown {
        std::fs::write(p, &md).unwrap();
    }
    println!("\n{md}");
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        annotate(&report);
    }
    let failed = report.checks.iter().filter(|c| !c.pass).count();
    if failed > 0 && args.fail {
        eprintln!("{failed} check(s) over target");
        std::process::exit(1);
    }
}

fn markdown(r: &Report) -> String {
    let mut s = format!(
        "### 性能基准（{}）\n\n仓库：{} 个提交、{} 个文件、{} 个分支和标签。判定阈值 = 目标 × {}。\n\n| 指标 | 目标 | 实测 | 结果 | 说明 |\n|---|---|---|---|---|\n",
        r.os, r.shape.commits, r.shape.files, r.shape.refs, r.slack
    );
    for c in &r.checks {
        s += &format!(
            "| {} | < {} ms | {:.1} ms | {} | {} |\n",
            c.label,
            c.target_ms,
            c.value_ms,
            if c.pass { "✅" } else { "❌" },
            c.note
        );
    }
    if !r.info.is_empty() {
        s += "\n| 参考数据 | 耗时 |\n|---|---|\n";
        for (what, v) in &r.info {
            s += &format!("| {what} | {v:.1} ms |\n");
        }
    }
    s
}

/// GitHub Actions annotations: one line per check, so results show on the
/// run page (and through the API) without opening the logs.
fn annotate(r: &Report) {
    for c in &r.checks {
        let level = if c.pass { "notice" } else { "error" };
        println!(
            "::{level} title=perf {}: {}::{:.1} ms (target {} ms x {}) {}",
            r.os,
            c.id,
            c.value_ms,
            c.target_ms,
            r.slack,
            c.note.replace('\n', " ")
        );
    }
}
