//! Persistent commit index behind the Log view.
//!
//! Every commit reachable from branches, remote branches, tags and HEAD is
//! stored in a per-repository SQLite file, in display order
//! (`git log --date-order`: children before parents, otherwise newest first).
//!
//! - **First open**: `git log` is streamed into the index. The first few
//!   hundred rows are readable almost at once; the rest follow in batches.
//! - **Later updates are incremental.** Commits that became reachable go in
//!   front; commits that are no longer reachable (amend, rebase, a deleted
//!   branch) are removed. New commits are never ancestors of old ones, so the
//!   result is still a valid topological order and old rows keep their place.
//! - **Reads are by row number.** The table's integer key is the display
//!   position; prepends use smaller keys and removals leave gaps, and a sorted
//!   in-memory list of keys turns a row number into a key range.
//!
//! Ref labels are not stored: they change far more often than history and
//! come from the same cheap `for-each-ref` call that detects changes.

mod record;
mod refs;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use glide_git_backend::{CancelToken, CommitSummary, GitError, Repo, RunOptions};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use ts_rs::TS;

pub use record::{hex_decode, hex_encode, Record, RecordParser, FORMAT};
pub use refs::{read_tips, RefTips};

/// Bump when the table layout changes; older files are rebuilt.
const SCHEMA_VERSION: &str = "2";
/// Rows written before the first status update, so the Log fills quickly.
const FIRST_BATCH: usize = 300;
const BATCH: usize = 20_000;
const CREATE_OID_INDEX: &str = "CREATE UNIQUE INDEX IF NOT EXISTS commits_oid ON commits(oid);";

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("log index database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("log index: {0}")]
    Io(#[from] std::io::Error),
}

impl IndexError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, IndexError::Git(GitError::Cancelled))
    }
}

pub type Result<T> = std::result::Result<T, IndexError>;

/// What the Log view needs to know to lay out its rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogStatus {
    pub root: String,
    /// Rows that can be read now.
    pub total: u32,
    /// Every reachable commit is indexed. `false` while the first build runs.
    pub complete: bool,
    /// Bumped whenever rows move or labels change; pages read under an older
    /// version must be read again.
    pub version: u64,
    /// The last update failed; the rows from before it are still readable.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogPage {
    pub start: u32,
    pub total: u32,
    pub version: u64,
    pub rows: Vec<CommitSummary>,
}

/// How [`LogIndex::update`] brought the index up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    Unchanged,
    /// Same commits, different ref labels.
    Labels,
    Incremental {
        added: usize,
        removed: usize,
    },
    Full,
}

#[derive(Default)]
struct State {
    loaded: bool,
    /// Table keys in display order (ascending).
    order: Vec<i64>,
    tips: RefTips,
    complete: bool,
    version: u64,
    error: Option<String>,
}

struct Writer {
    conn: Connection,
    /// (name, email) → id; loaded lazily, dropped on rollback.
    authors: Option<HashMap<(String, String), i64>>,
}

impl Writer {
    fn author_id(&mut self, name: &str, email: &str) -> Result<i64> {
        if self.authors.is_none() {
            let mut map = HashMap::new();
            let mut stmt = self.conn.prepare("SELECT id, name, email FROM authors")?;
            let rows = stmt.query_map([], |r| Ok(((r.get(1)?, r.get(2)?), r.get(0)?)))?;
            for row in rows {
                let (k, v) = row?;
                map.insert(k, v);
            }
            self.authors = Some(map);
        }
        let authors = self.authors.as_mut().expect("loaded above");
        if let Some(&id) = authors.get(&(name.to_string(), email.to_string())) {
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO authors(name, email) VALUES (?1, ?2)",
            params![name, email],
        )?;
        let id = self.conn.last_insert_rowid();
        authors.insert((name.to_string(), email.to_string()), id);
        Ok(id)
    }

    /// Insert rows at consecutive keys from `first`; returns the keys used.
    fn insert(&mut self, rows: impl IntoIterator<Item = Record>, first: i64) -> Result<Vec<i64>> {
        let mut keys = Vec::new();
        for (i, r) in rows.into_iter().enumerate() {
            let pos = first + i as i64;
            let author = self.author_id(&r.author_name, &r.author_email)?;
            let mut stmt = self.conn.prepare_cached(
                "INSERT OR IGNORE INTO commits(pos, oid, parents, author, time, subject) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            if stmt.execute(params![
                pos,
                r.oid,
                r.parents,
                author,
                r.author_time,
                r.subject
            ])? == 1
            {
                keys.push(pos);
            }
        }
        Ok(keys)
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }
}

fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub struct LogIndex {
    root: String,
    path: PathBuf,
    writer: Mutex<Writer>,
    reader: Mutex<Connection>,
    state: RwLock<State>,
}

impl LogIndex {
    /// Open (or create) the index for the repository at `root`, stored under
    /// `cache_dir`. Cheap: the index is loaded by the first [`update`](Self::update).
    pub fn open(cache_dir: &Path, root: &Path) -> Result<Self> {
        std::fs::create_dir_all(cache_dir)?;
        let root = root.to_string_lossy().into_owned();
        let path = cache_dir.join(format!("{:016x}.sqlite", fnv1a(root.as_bytes())));

        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // 32 MiB page cache (negative = KiB) keeps index builds off the disk.
        conn.pragma_update(None, "cache_size", -32_000)?;
        init_schema(&conn, &root)?;

        let reader = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        reader.busy_timeout(Duration::from_secs(5))?;

        Ok(Self {
            root,
            path,
            writer: Mutex::new(Writer {
                conn,
                authors: None,
            }),
            reader: Mutex::new(reader),
            state: RwLock::new(State {
                version: 1,
                ..State::default()
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn status(&self) -> LogStatus {
        let st = self.read_state();
        LogStatus {
            root: self.root.clone(),
            total: st.order.len() as u32,
            complete: st.complete,
            version: st.version,
            error: st.error.clone(),
        }
    }

    /// Rows `start..start + count` (clamped to what exists).
    pub fn page(&self, start: u32, count: u32) -> Result<LogPage> {
        let st = self.read_state();
        let total = st.order.len();
        let s = (start as usize).min(total);
        let e = s.saturating_add(count as usize).min(total);
        let mut rows = Vec::with_capacity(e - s);
        if s < e {
            let db = lock(&self.reader);
            let mut stmt = db.prepare_cached(
                "SELECT c.oid, c.parents, a.name, a.email, c.time, c.subject \
                 FROM commits c JOIN authors a ON a.id = c.author \
                 WHERE c.pos BETWEEN ?1 AND ?2 ORDER BY c.pos",
            )?;
            let it = stmt.query_map([st.order[s], st.order[e - 1]], |r| {
                let oid: Vec<u8> = r.get(0)?;
                let parents: Vec<u8> = r.get(1)?;
                Ok(CommitSummary {
                    oid: hex_encode(&oid),
                    parents: parents.chunks(oid.len().max(1)).map(hex_encode).collect(),
                    author_name: r.get(2)?,
                    author_email: r.get(3)?,
                    author_time: r.get(4)?,
                    subject: r.get(5)?,
                    refs: Vec::new(),
                })
            })?;
            for row in it {
                let mut c = row?;
                if let Some(l) = st.tips.labels.get(&c.oid) {
                    c.refs = l.clone();
                }
                rows.push(c);
            }
        }
        Ok(LogPage {
            start: s as u32,
            total: total as u32,
            version: st.version,
            rows,
        })
    }

    /// Row number of a commit (full hex id), if it is indexed.
    pub fn row_of(&self, oid: &str) -> Result<Option<u32>> {
        let Some(bytes) = hex_decode(oid) else {
            return Ok(None);
        };
        let st = self.read_state();
        let pos: Option<i64> = lock(&self.reader)
            .query_row("SELECT pos FROM commits WHERE oid = ?1", [bytes], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(pos.and_then(|p| st.order.binary_search(&p).ok().map(|i| i as u32)))
    }

    /// Bring the index in line with the repository's current refs.
    /// Blocking; `on_status` is called whenever readable rows change.
    pub fn update(
        &self,
        repo: &Repo,
        cancel: &CancelToken,
        on_status: &dyn Fn(LogStatus),
    ) -> Result<UpdateKind> {
        let mut w = lock(&self.writer);
        let result = self.update_locked(&mut w, repo, cancel, on_status);
        {
            let mut st = self.write_state();
            st.error = match &result {
                Err(e) if !e.is_cancelled() => Some(e.to_string()),
                Err(_) => st.error.take(),
                Ok(_) => None,
            };
        }
        on_status(self.status());
        result
    }

    fn update_locked(
        &self,
        w: &mut Writer,
        repo: &Repo,
        cancel: &CancelToken,
        on_status: &dyn Fn(LogStatus),
    ) -> Result<UpdateKind> {
        if !self.read_state().loaded {
            self.load(w)?;
            on_status(self.status());
        }
        let new = read_tips(repo)?;
        let (old_tips, complete) = {
            let st = self.read_state();
            (st.tips.tips.clone(), st.complete)
        };
        if complete && old_tips == new.tips {
            let mut st = self.write_state();
            if st.tips.labels == new.labels {
                return Ok(UpdateKind::Unchanged);
            }
            st.tips = new;
            st.version += 1;
            return Ok(UpdateKind::Labels);
        }
        if complete {
            if let Some(kind) = self.incremental(w, repo, &old_tips, &new, cancel)? {
                return Ok(kind);
            }
        }
        self.full_build(w, repo, new, cancel, on_status)?;
        Ok(UpdateKind::Full)
    }

    /// Read what a previous run left on disk. An unfinished build is discarded.
    fn load(&self, w: &mut Writer) -> Result<()> {
        let complete = get_meta(&w.conn, "complete")?.as_deref() == Some("1");
        let mut order = Vec::new();
        let mut tips = RefTips::default();
        if complete {
            let mut stmt = w.conn.prepare("SELECT pos FROM commits ORDER BY pos")?;
            for pos in stmt.query_map([], |r| r.get::<_, i64>(0))? {
                order.push(pos?);
            }
            tips.tips = get_meta(&w.conn, "tips")?
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
        }
        let mut st = self.write_state();
        st.loaded = true;
        st.order = order;
        st.tips = tips;
        st.complete = complete;
        st.version += 1;
        Ok(())
    }

    /// Apply the difference between the old and new tips. `None` when that is
    /// not possible (e.g. old tips were garbage-collected) and a rebuild is needed.
    fn incremental(
        &self,
        w: &mut Writer,
        repo: &Repo,
        old: &[String],
        new: &RefTips,
        cancel: &CancelToken,
    ) -> Result<Option<UpdateKind>> {
        // No longer reachable: in the old history but not the new one.
        let out = match repo.cli().run(
            repo.root(),
            &["rev-list", "--stdin"],
            RunOptions::read()
                .stdin(rev_input(old, &new.tips))
                .cancel(cancel.clone()),
        ) {
            Ok(out) => out,
            Err(GitError::Failed { message, .. }) => {
                // Expected after `git gc` pruned an old tip.
                tracing::info!("log index: rebuilding, incremental update failed: {message}");
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let removed: Vec<Vec<u8>> = out.stdout_str().lines().filter_map(hex_decode).collect();

        // Newly reachable, in display order.
        let mut added = Vec::new();
        let mut parser = RecordParser::default();
        match repo.cli().run_streaming(
            repo.root(),
            &["log", "--date-order", FORMAT, "--stdin"],
            RunOptions::read()
                .stdin(rev_input(&new.tips, old))
                .cancel(cancel.clone()),
            |chunk| {
                parser.feed(chunk, |r| added.push(r));
                true
            },
        ) {
            Ok(()) => {}
            Err(GitError::Failed { message, .. }) => {
                // Expected after `git gc` pruned an old tip.
                tracing::info!("log index: rebuilding, incremental update failed: {message}");
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        }

        let kind = UpdateKind::Incremental {
            added: added.len(),
            removed: removed.len(),
        };
        let first = self.read_state().order.first().copied().unwrap_or(0);
        let base = first - added.len() as i64;
        let tips_text = new.tips.join("\n");
        self.apply(
            w,
            |w| {
                let mut gone = HashSet::new();
                for oid in &removed {
                    let pos: Option<i64> = w
                        .conn
                        .prepare_cached("SELECT pos FROM commits WHERE oid = ?1")?
                        .query_row([oid], |r| r.get(0))
                        .optional()?;
                    if let Some(pos) = pos {
                        w.conn
                            .prepare_cached("DELETE FROM commits WHERE pos = ?1")?
                            .execute([pos])?;
                        gone.insert(pos);
                    }
                }
                let keys = w.insert(added, base)?;
                w.set_meta("tips", &tips_text)?;
                Ok((gone, keys))
            },
            |st, (gone, keys)| {
                if !gone.is_empty() {
                    st.order.retain(|p| !gone.contains(p));
                }
                st.order.splice(0..0, keys);
                st.tips = new.clone();
                st.version += 1;
            },
        )?;
        Ok(Some(kind))
    }

    fn full_build(
        &self,
        w: &mut Writer,
        repo: &Repo,
        new: RefTips,
        cancel: &CancelToken,
        on_status: &dyn Fn(LogStatus),
    ) -> Result<()> {
        let tips = new.tips.clone();
        self.apply(
            w,
            |w| {
                w.conn.execute_batch(
                    // The oid index is rebuilt once at the end: maintaining it
                    // row by row would make the build several times slower.
                    "DROP INDEX IF EXISTS commits_oid; \
                     DELETE FROM commits; DELETE FROM authors; \
                     DELETE FROM meta WHERE key IN ('complete', 'tips');",
                )?;
                w.authors = Some(HashMap::new());
                Ok(())
            },
            |st, ()| {
                st.order.clear();
                st.complete = false;
                st.tips = new;
                st.version += 1;
            },
        )?;
        on_status(self.status());

        if !tips.is_empty() {
            // git runs ahead on its own thread while rows are written here,
            // so the build takes about as long as `git log` alone.
            let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<Record>>(4);
            let input = rev_input(&tips, &[]);
            std::thread::scope(|scope| -> Result<()> {
                let reader = scope.spawn(move || {
                    let mut parser = RecordParser::default();
                    let mut batch = Vec::new();
                    let mut limit = FIRST_BATCH;
                    let res = repo.cli().run_streaming(
                        repo.root(),
                        &["log", "--date-order", FORMAT, "--stdin"],
                        RunOptions::read().stdin(input).cancel(cancel.clone()),
                        |chunk| {
                            parser.feed(chunk, |r| batch.push(r));
                            if batch.len() < limit {
                                return true;
                            }
                            limit = BATCH;
                            // Fails once the writer gave up; that stops git.
                            tx.send(std::mem::take(&mut batch)).is_ok()
                        },
                    );
                    if res.is_ok() && !batch.is_empty() {
                        let _ = tx.send(batch);
                    }
                    res
                });

                let mut next = 0i64;
                let written = rx.iter().try_for_each(|mut rows| {
                    self.append(w, &mut rows, &mut next)?;
                    on_status(self.status());
                    Ok::<_, IndexError>(())
                });
                drop(rx);
                let streamed = reader.join().expect("git log reader panicked");
                written?;
                Ok(streamed?)
            })?;
        }

        let tips_text = tips.join("\n");
        self.apply(
            w,
            |w| {
                w.set_meta("tips", &tips_text)?;
                w.conn.execute_batch(CREATE_OID_INDEX)?;
                w.set_meta("complete", "1")
            },
            |st, ()| st.complete = true,
        )
    }

    fn append(&self, w: &mut Writer, batch: &mut Vec<Record>, next: &mut i64) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let first = *next;
        *next += batch.len() as i64;
        let rows = std::mem::take(batch);
        self.apply(
            w,
            |w| w.insert(rows, first),
            |st, keys| st.order.extend(keys),
        )
    }

    /// Run `f` in a transaction and publish its effect on the in-memory state
    /// atomically with the commit, so readers never see the two disagree.
    fn apply<T>(
        &self,
        w: &mut Writer,
        f: impl FnOnce(&mut Writer) -> Result<T>,
        publish: impl FnOnce(&mut State, T),
    ) -> Result<()> {
        w.conn.execute_batch("BEGIN IMMEDIATE")?;
        let value = match f(w) {
            Ok(v) => v,
            Err(e) => {
                let _ = w.conn.execute_batch("ROLLBACK");
                w.authors = None;
                return Err(e);
            }
        };
        let mut st = self.write_state();
        if let Err(e) = w.conn.execute_batch("COMMIT") {
            let _ = w.conn.execute_batch("ROLLBACK");
            w.authors = None;
            return Err(e.into());
        }
        publish(&mut st, value);
        Ok(())
    }

    fn read_state(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_state(&self) -> RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(|e| e.into_inner())
    }
}

fn init_schema(conn: &Connection, root: &str) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    let schema = get_meta(conn, "schema")?;
    let stored_root = get_meta(conn, "root")?;
    if schema.as_deref() != Some(SCHEMA_VERSION) || stored_root.as_deref() != Some(root) {
        conn.execute_batch(
            "DROP TABLE IF EXISTS commits; DROP TABLE IF EXISTS authors; DELETE FROM meta;",
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS authors(
             id INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             email TEXT NOT NULL,
             UNIQUE(name, email));
         CREATE TABLE IF NOT EXISTS commits(
             pos INTEGER PRIMARY KEY,
             oid BLOB NOT NULL,
             parents BLOB NOT NULL,
             author INTEGER NOT NULL,
             time INTEGER NOT NULL,
             subject TEXT NOT NULL);",
    )?;
    conn.execute_batch(CREATE_OID_INDEX)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema', ?1), ('root', ?2)",
        params![SCHEMA_VERSION, root],
    )?;
    Ok(())
}

/// `--stdin` input for rev-list/log: `include` tips, minus history of `exclude`.
fn rev_input(include: &[String], exclude: &[String]) -> Vec<u8> {
    let mut s = String::new();
    for t in include {
        s.push_str(t);
        s.push('\n');
    }
    for t in exclude {
        s.push('^');
        s.push_str(t);
        s.push('\n');
    }
    s.into_bytes()
}

/// Stable across runs and Rust versions (unlike `DefaultHasher`).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
