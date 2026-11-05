//! A synthetic repository, the same on every run: a long history with
//! regular merges, a wide working tree and hundreds of branches and tags.
//! Built with `git fast-import`, which takes seconds even for a million
//! commits.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Shape {
    pub commits: u32,
    pub files: u32,
    pub refs: u32,
}

const AUTHORS: [&str; 12] = [
    "Alex Chen",
    "Sam Lee",
    "Min",
    "Robin Park",
    "Kai Wu",
    "Jo Smith",
    "Noa Levi",
    "Ana Silva",
    "Lin Zhao",
    "Tom Berg",
    "Eva Novak",
    "Raj Iyer",
];
/// A merge every this many commits (a short side branch of two commits).
const MERGE_EVERY: u32 = 100;
const START_TIME: u64 = 1_500_000_000;

type W = BufWriter<std::process::ChildStdin>;

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The wide part of the tree: written once, never changed afterwards.
fn path_of(i: u32) -> String {
    format!(
        "assets/module{:02}/part{:03}/file{i:06}.txt",
        i % 40,
        (i / 40) % 100
    )
}

/// Files the history keeps changing. Kept apart from the wide part so each
/// commit rewrites only a few small trees (generation is twice as fast).
const HOT_FILES: u32 = 400;
fn hot_path(i: u32) -> String {
    format!("src/mod{:02}/file{:03}.txt", i % 20, i % HOT_FILES)
}

/// Create the repository at `dir` (which must not exist) and check it out.
pub fn generate(dir: &Path, shape: Shape) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Bench"]);
    git(dir, &["config", "user.email", "bench@example.com"]);
    // Keep timings about GlideGit, not about background gc.
    git(dir, &["config", "gc.auto", "0"]);
    git(dir, &["config", "maintenance.auto", "false"]);

    let mut child = Command::new("git")
        .args(["fast-import", "--quiet", "--done"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .spawn()
        .expect("git fast-import starts");
    let mut w = BufWriter::with_capacity(1 << 20, child.stdin.take().unwrap());
    let mut mark = 0u32;
    let mut next_mark = || {
        mark += 1;
        mark
    };

    // The initial tree: every file starts with the same content (one blob).
    let blob = next_mark();
    let content = "initial content\n";
    write!(w, "blob\nmark :{blob}\ndata {}\n{content}\n", content.len()).unwrap();

    let mut time = START_TIME;
    let mut commit = |w: &mut W,
                      branch: &str,
                      mark: u32,
                      from: Option<u32>,
                      merge: Option<u32>,
                      n: u32,
                      body: &mut dyn FnMut(&mut W)| {
        time += 60;
        let author = AUTHORS[(n as usize * 7) % AUTHORS.len()];
        let email = author.to_lowercase().replace(' ', ".");
        let msg = format!("Change {n}: update module {}\n", n % 40);
        write!(
            w,
            "commit refs/heads/{branch}\nmark :{mark}\nauthor {author} <{email}@example.com> {time} +0000\ncommitter {author} <{email}@example.com> {time} +0000\ndata {}\n{msg}",
            msg.len()
        )
        .unwrap();
        if let Some(f) = from {
            writeln!(w, "from :{f}").unwrap();
        }
        if let Some(m) = merge {
            writeln!(w, "merge :{m}").unwrap();
        }
        body(w);
        writeln!(w).unwrap();
    };

    let root = next_mark();
    commit(&mut w, "main", root, None, None, 0, &mut |w| {
        for i in 0..shape.files {
            writeln!(w, "M 100644 :{blob} {}", path_of(i)).unwrap();
        }
        for i in 0..HOT_FILES {
            writeln!(w, "M 100644 :{blob} {}", hot_path(i)).unwrap();
        }
    });
    let mut marks = vec![root];
    let mut head = root;
    let change = |w: &mut W, n: u32| {
        let data = format!("content of change {n}\n");
        write!(
            w,
            "M 100644 inline {}\ndata {}\n{data}",
            hot_path(n.wrapping_mul(2_654_435_761) % HOT_FILES),
            data.len()
        )
        .unwrap();
    };
    let mut n = 1;
    while n < shape.commits {
        if n % MERGE_EVERY == 0 && n + 3 <= shape.commits {
            // A side branch of two commits off `head`, merged back.
            let a = next_mark();
            commit(&mut w, "side", a, Some(head), None, n, &mut |w| {
                change(w, n)
            });
            let b = next_mark();
            commit(&mut w, "side", b, Some(a), None, n + 1, &mut |w| {
                change(w, n + 1)
            });
            let c = next_mark();
            commit(&mut w, "main", c, Some(head), None, n + 2, &mut |w| {
                change(w, n + 2)
            });
            let m = next_mark();
            commit(&mut w, "main", m, Some(c), Some(b), n + 3, &mut |_| {});
            marks.extend([a, b, c, m]);
            head = m;
            n += 4;
        } else {
            let c = next_mark();
            commit(&mut w, "main", c, Some(head), None, n, &mut |w| {
                change(w, n)
            });
            marks.push(c);
            head = c;
            n += 1;
        }
    }
    // Branches and tags spread over the history.
    for i in 0..shape.refs {
        let at = marks[(i as usize * 7919) % marks.len()];
        let name = if i % 2 == 0 {
            format!("refs/heads/feature/topic-{i:04}")
        } else {
            format!("refs/tags/v{}.{}", i / 100, i % 100)
        };
        write!(w, "reset {name}\nfrom :{at}\n\n").unwrap();
    }
    write!(w, "reset refs/heads/side\nfrom :{head}\n\ndone\n").unwrap();
    w.flush().unwrap();
    drop(w);
    assert!(child.wait().unwrap().success(), "git fast-import failed");
    git(dir, &["checkout", "-q", "-f", "main"]);
    // As in a clone: refs packed into one file, not 800 loose ones.
    git(dir, &["pack-refs", "--all"]);
}

/// Reuse the repository at `dir` when it was made with the same shape.
/// Bump when the generated history changes, so cached repositories (CI
/// caches them) are rebuilt.
pub const LAYOUT: u32 = 3;

/// Reuse the repository at `dir` when it was made with the same shape and
/// layout. The commit-graph is written here too (once): the app writes one
/// after its first index build, and it takes a while on a big history.
pub fn ensure(dir: &Path, shape: Shape) -> bool {
    let marker = dir.join(".git").join("glidegit-bench.json");
    let want = serde_json::json!({ "layout": LAYOUT, "shape": shape }).to_string();
    if std::fs::read_to_string(&marker).ok().as_deref() == Some(want.as_str()) {
        // Undo whatever an earlier run left in the working tree.
        git(dir, &["reset", "-q", "--hard", "main"]);
        return false;
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir).unwrap();
    }
    generate(dir, shape);
    git(
        dir,
        &["commit-graph", "write", "--reachable", "--changed-paths"],
    );
    std::fs::write(marker, want).unwrap();
    true
}

pub fn changed_path(shape: Shape) -> String {
    path_of(shape.files / 2)
}
