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

mod filter;
mod graph;
mod record;
mod refs;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use glide_git_backend::{CancelToken, CommitSummary, GitError, Repo, RunOptions};
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use ts_rs::TS;

pub use filter::{AuthorCount, LogFilter};
pub use graph::{GraphRow, Lane, Layout};
pub use record::{hex_decode, hex_encode, Record, RecordParser, FORMAT};
pub use refs::{read_tips, RefTips};

/// Bump when the table layout changes; older files are rebuilt.
const SCHEMA_VERSION: &str = "2";
/// Rows written before the first status update, so the Log fills quickly.
const FIRST_BATCH: usize = 300;
const BATCH: usize = 20_000;
/// Rows between saved layout states; a page replays at most this many extra rows.
const CHECKPOINT_EVERY: usize = 256;
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
    /// Graph drawing data, one entry per row; empty when the rows have no
    /// graph (a filter left holes in the history).
    pub graph: Vec<GraphRow>,
}

/// The rows of a filtered Log, computed once per filter and index version.
struct View {
    filter: LogFilter,
    version: u64,
    /// Table keys of the matching rows, ascending.
    positions: Vec<i64>,
    /// Layout states for the view's own graph, when it has one.
    checkpoints: Option<Vec<Checkpoint>>,
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
    /// Until the index is loaded: its size and labels as saved last time, so
    /// the first pages can be served straight from the database.
    early: Option<Early>,
    /// Table keys in display order (ascending).
    order: Vec<i64>,
    /// Graph layout state before selected rows, ascending by table key.
    checkpoints: Vec<Checkpoint>,
    tips: RefTips,
    complete: bool,
    version: u64,
    error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Early {
    total: u32,
    labels: HashMap<String, Vec<String>>,
}

/// Before the index is loaded, pages up to this row get a graph laid out
/// from the top; deeper ones come without graph until loading finishes.
const EARLY_GRAPH_ROWS: usize = 20_000;

/// The graph layout state before the row with table key `pos`.
#[derive(Debug, Clone)]
struct Checkpoint {
    pos: i64,
    lanes: Vec<Lane>,
}

/// Lays out rows in display order, saving a checkpoint every
/// [`CHECKPOINT_EVERY`] rows.
#[derive(Default)]
struct Tracker {
    layout: Layout,
    row: usize,
    /// The first row's checkpoint already exists (resuming from it).
    resumed: bool,
}

impl Tracker {
    fn resume(cp: &Checkpoint) -> Self {
        Self {
            layout: Layout::from_lanes(cp.lanes.clone()),
            row: 0,
            resumed: true,
        }
    }

    fn feed(&mut self, pos: i64, oid: &[u8], parents: &[u8], out: &mut Vec<Checkpoint>) {
        if self.resumed {
            self.resumed = false;
        } else if self.row.is_multiple_of(CHECKPOINT_EVERY) {
            out.push(Checkpoint {
                pos,
                lanes: self.layout.lanes().to_vec(),
            });
        }
        self.layout.step(oid, parents, None);
        self.row += 1;
    }
}

fn blob<'a>(v: ValueRef<'a>) -> &'a [u8] {
    match v {
        ValueRef::Blob(b) => b,
        _ => &[],
    }
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
    /// Most recent filtered view.
    view: Mutex<Option<Arc<View>>>,
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

        // What a complete index on disk holds, for the first pages.
        let early = if get_meta(&conn, "complete")?.as_deref() == Some("1") {
            get_meta(&conn, "count")?
                .and_then(|n| n.parse().ok())
                .map(|total| Early {
                    total,
                    labels: get_meta(&conn, "labels")
                        .ok()
                        .flatten()
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default(),
                })
        } else {
            None
        };
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
                early,
                ..State::default()
            }),
            view: Mutex::new(None),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn status(&self) -> LogStatus {
        let st = self.read_state();
        if let (false, Some(e)) = (st.loaded, &st.early) {
            return LogStatus {
                root: self.root.clone(),
                total: e.total,
                complete: true,
                version: st.version,
                error: None,
            };
        }
        LogStatus {
            root: self.root.clone(),
            total: st.order.len() as u32,
            complete: st.complete,
            version: st.version,
            error: st.error.clone(),
        }
    }

    /// Rows `start..start + count` (clamped to what exists), with their graph.
    pub fn page(&self, start: u32, count: u32) -> Result<LogPage> {
        let st = self.read_state();
        if let (false, Some(e)) = (st.loaded, &st.early) {
            return self.early_page(&st, e, start, count);
        }
        self.read_page(&st, &st.order, Some(&st.checkpoints), start, count)
    }

    /// A page read before the index is loaded: rows by offset straight from
    /// the table (its key order is the display order).
    fn early_page(&self, st: &State, e: &Early, start: u32, count: u32) -> Result<LogPage> {
        let total = e.total as usize;
        let s = (start as usize).min(total);
        let end = s.saturating_add(count as usize).min(total);
        let with_graph = end <= EARLY_GRAPH_ROWS;
        let from = if with_graph { 0 } else { s };
        let mut rows = Vec::new();
        let mut graph = Vec::new();
        let mut layout = Layout::new();
        let db = lock(&self.reader);
        let mut stmt = db.prepare_cached(
            "SELECT c.pos, c.oid, c.parents, a.name, a.email, c.time, c.subject \
             FROM commits c JOIN authors a ON a.id = c.author \
             ORDER BY c.pos LIMIT ?1 OFFSET ?2",
        )?;
        let mut it = stmt.query([(end - from) as i64, from as i64])?;
        let mut row = from;
        while let Some(r) = it.next()? {
            let oid = blob(r.get_ref(1)?);
            let parents = blob(r.get_ref(2)?);
            if row < s {
                layout.step(oid, parents, None);
                row += 1;
                continue;
            }
            if with_graph {
                let mut g = GraphRow::default();
                layout.step(oid, parents, Some(&mut g));
                graph.push(g);
            }
            let hex = hex_encode(oid);
            rows.push(CommitSummary {
                refs: e.labels.get(&hex).cloned().unwrap_or_default(),
                oid: hex,
                parents: parents.chunks(oid.len().max(1)).map(hex_encode).collect(),
                author_name: r.get(3)?,
                author_email: r.get(4)?,
                author_time: r.get(5)?,
                subject: r.get(6)?,
            });
            row += 1;
        }
        Ok(LogPage {
            start: s as u32,
            total: total as u32,
            version: st.version,
            rows,
            graph,
        })
    }

    /// Like [`page`](Self::page), for the rows matching `filter`.
    pub fn page_filtered(
        &self,
        repo: &Repo,
        filter: &LogFilter,
        cancel: &CancelToken,
        start: u32,
        count: u32,
    ) -> Result<LogPage> {
        if filter.is_empty() {
            return self.page(start, count);
        }
        let view = self.view(repo, filter, cancel)?;
        let st = self.read_state();
        let mut page = self.read_page(
            &st,
            &view.positions,
            view.checkpoints.as_deref(),
            start,
            count,
        )?;
        // Report the version the view was computed for, so a newer index
        // version makes the UI ask again (and the view get recomputed).
        page.version = view.version;
        Ok(page)
    }

    /// Row number of a commit (full hex id) in the full Log, if indexed.
    pub fn row_of(&self, oid: &str) -> Result<Option<u32>> {
        let st = self.read_state();
        let Some(pos) = self.pos_of(oid)? else {
            return Ok(None);
        };
        if !st.loaded && st.early.is_some() {
            // Not loaded yet: the row is the number of keys before it.
            let n: i64 = lock(&self.reader).query_row(
                "SELECT COUNT(*) FROM commits WHERE pos < ?1",
                [pos],
                |r| r.get(0),
            )?;
            return Ok(Some(n as u32));
        }
        Ok(st.order.binary_search(&pos).ok().map(|i| i as u32))
    }

    /// Row number of a commit in the Log filtered by `filter`.
    pub fn row_of_filtered(
        &self,
        repo: &Repo,
        filter: &LogFilter,
        cancel: &CancelToken,
        oid: &str,
    ) -> Result<Option<u32>> {
        if filter.is_empty() {
            return self.row_of(oid);
        }
        let view = self.view(repo, filter, cancel)?;
        let pos = self.pos_of(oid)?;
        Ok(pos.and_then(|p| view.positions.binary_search(&p).ok().map(|i| i as u32)))
    }

    /// Authors whose `Name <email>` contains `query`, most commits first.
    pub fn authors(&self, query: &str, limit: u32) -> Result<Vec<AuthorCount>> {
        let db = lock(&self.reader);
        let mut stmt = db.prepare_cached(
            "SELECT a.name, a.email, count(*) AS n \
             FROM commits c JOIN authors a ON a.id = c.author \
             WHERE (a.name || ' <' || a.email || '>') LIKE ?1 ESCAPE '\\' \
             GROUP BY c.author ORDER BY n DESC LIMIT ?2",
        )?;
        let pattern = filter::like_pattern(query.trim());
        let rows = stmt.query_map(params![pattern, limit], |r| {
            Ok(AuthorCount {
                name: r.get(0)?,
                email: r.get(1)?,
                commits: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn pos_of(&self, oid: &str) -> Result<Option<i64>> {
        let Some(bytes) = hex_decode(oid) else {
            return Ok(None);
        };
        Ok(lock(&self.reader)
            .query_row("SELECT pos FROM commits WHERE oid = ?1", [bytes], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// The filtered view for `filter`, reusing the last one if it is current.
    fn view(&self, repo: &Repo, filter: &LogFilter, cancel: &CancelToken) -> Result<Arc<View>> {
        let (version, tips) = {
            let st = self.read_state();
            (st.version, st.tips.tips.clone())
        };
        if let Some(v) = lock(&self.view).as_ref() {
            if v.version == version && v.filter == *filter {
                return Ok(v.clone());
            }
        }
        let allowed = filter.git_set(repo, &tips, cancel)?;
        let graph = filter.keeps_graph();
        let (cond, params) = filter.sql();

        let st = self.read_state();
        let mut positions = Vec::new();
        let mut checkpoints = Vec::new();
        let mut tracker = Tracker::default();
        {
            let db = lock(&self.reader);
            let mut stmt = db.prepare(&format!(
                "SELECT c.pos, c.oid, c.parents FROM commits c JOIN authors a ON a.id = c.author \
                 WHERE {cond} ORDER BY c.pos"
            ))?;
            let mut it = stmt.query(rusqlite::params_from_iter(params))?;
            let mut n = 0u32;
            while let Some(r) = it.next()? {
                n = n.wrapping_add(1);
                if n.is_multiple_of(16_384) && cancel.is_cancelled() {
                    return Err(GitError::Cancelled.into());
                }
                let oid = blob(r.get_ref(1)?);
                if allowed.as_ref().is_some_and(|set| !set.contains(oid)) {
                    continue;
                }
                let pos: i64 = r.get(0)?;
                if graph {
                    tracker.feed(pos, oid, blob(r.get_ref(2)?), &mut checkpoints);
                }
                positions.push(pos);
            }
        }
        let view = Arc::new(View {
            filter: filter.clone(),
            version: st.version,
            positions,
            checkpoints: graph.then_some(checkpoints),
        });
        *lock(&self.view) = Some(view.clone());
        Ok(view)
    }

    /// Rows `start..start + count` of the row list `positions` (ascending
    /// table keys). With `checkpoints` the graph is laid out from the
    /// nearest one; rows between that are not in the list are skipped.
    fn read_page(
        &self,
        st: &State,
        positions: &[i64],
        checkpoints: Option<&[Checkpoint]>,
        start: u32,
        count: u32,
    ) -> Result<LogPage> {
        let total = positions.len();
        let s = (start as usize).min(total);
        let e = s.saturating_add(count as usize).min(total);
        let mut rows = Vec::with_capacity(e - s);
        let mut graph = Vec::new();
        let db = lock(&self.reader);
        let to_summary = |r: &rusqlite::Row, oid: &[u8], parents: &[u8]| -> rusqlite::Result<_> {
            let hex = hex_encode(oid);
            Ok(CommitSummary {
                refs: st.tips.labels.get(&hex).cloned().unwrap_or_default(),
                oid: hex,
                parents: parents.chunks(oid.len().max(1)).map(hex_encode).collect(),
                author_name: r.get(3)?,
                author_email: r.get(4)?,
                author_time: r.get(5)?,
                subject: r.get(6)?,
            })
        };
        const COLUMNS: &str = "SELECT c.pos, c.oid, c.parents, a.name, a.email, c.time, c.subject \
                               FROM commits c JOIN authors a ON a.id = c.author";

        match checkpoints {
            Some(cps) if s < e => {
                let (first, last) = (positions[s], positions[e - 1]);
                // Resume the layout from the nearest saved state above the page.
                let (mut layout, from) = match cps.partition_point(|c| c.pos <= first) {
                    0 => (Layout::new(), positions[0]),
                    i => (Layout::from_lanes(cps[i - 1].lanes.clone()), cps[i - 1].pos),
                };
                let mut stmt = db.prepare_cached(&format!(
                    "{COLUMNS} WHERE c.pos BETWEEN ?1 AND ?2 ORDER BY c.pos"
                ))?;
                let mut it = stmt.query([from, last])?;
                let full = positions.len() == st.order.len();
                while let Some(r) = it.next()? {
                    let pos: i64 = r.get(0)?;
                    if !full && positions.binary_search(&pos).is_err() {
                        continue;
                    }
                    let oid = blob(r.get_ref(1)?);
                    let parents = blob(r.get_ref(2)?);
                    if pos < first {
                        layout.step(oid, parents, None);
                        continue;
                    }
                    let mut g = GraphRow::default();
                    layout.step(oid, parents, Some(&mut g));
                    graph.push(g);
                    rows.push(to_summary(r, oid, parents)?);
                }
            }
            _ => {
                // No graph: the rows may be far apart, read them one by one.
                let mut stmt = db.prepare_cached(&format!("{COLUMNS} WHERE c.pos = ?1"))?;
                for &pos in &positions[s..e] {
                    let mut it = stmt.query([pos])?;
                    if let Some(r) = it.next()? {
                        let oid = blob(r.get_ref(1)?);
                        let parents = blob(r.get_ref(2)?);
                        rows.push(to_summary(r, oid, parents)?);
                    }
                }
            }
        }
        Ok(LogPage {
            start: s as u32,
            total: total as u32,
            version: st.version,
            rows,
            graph,
        })
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
        if result.is_ok() {
            self.save_summary(&w);
        }
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

    /// Remember the row count and labels, for the first pages next time.
    fn save_summary(&self, w: &Writer) {
        let (count, labels) = {
            let st = self.read_state();
            (
                st.order.len(),
                serde_json::to_string(&st.tips.labels).unwrap_or_default(),
            )
        };
        let _ = w.set_meta("count", &count.to_string());
        let _ = w.set_meta("labels", &labels);
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
        let mut checkpoints = Vec::new();
        let mut tips = RefTips::default();
        if complete {
            let mut tracker = Tracker::default();
            let mut stmt = w
                .conn
                .prepare("SELECT pos, oid, parents FROM commits ORDER BY pos")?;
            let mut it = stmt.query([])?;
            while let Some(r) = it.next()? {
                let pos: i64 = r.get(0)?;
                tracker.feed(
                    pos,
                    blob(r.get_ref(1)?),
                    blob(r.get_ref(2)?),
                    &mut checkpoints,
                );
                order.push(pos);
            }
            tips.tips = get_meta(&w.conn, "tips")?
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect();
        }
        let mut st = self.write_state();
        st.loaded = true;
        st.early = None;
        st.order = order;
        st.checkpoints = checkpoints;
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
                let checkpoints = self.relayout(&w.conn, &gone, &keys)?;
                Ok((gone, keys, checkpoints))
            },
            |st, (gone, keys, checkpoints)| {
                if !gone.is_empty() {
                    st.order.retain(|p| !gone.contains(p));
                }
                st.order.splice(0..0, keys);
                if let Some(c) = checkpoints {
                    st.checkpoints = c;
                }
                st.tips = new.clone();
                st.version += 1;
            },
        )?;
        Ok(Some(kind))
    }

    /// New checkpoints after `gone` rows were removed and `added` rows put
    /// in front (`None`: nothing moved). Reads the uncommitted transaction.
    ///
    /// Lays the rows out again from the first affected one, but only until
    /// the state matches an old checkpoint below every change: from there on
    /// the layout is the same as before, so the rest is reused.
    fn relayout(
        &self,
        conn: &Connection,
        gone: &HashSet<i64>,
        added: &[i64],
    ) -> Result<Option<Vec<Checkpoint>>> {
        if gone.is_empty() && added.is_empty() {
            return Ok(None);
        }
        let st = self.read_state();
        let old = &st.checkpoints;
        let settled_after = gone.iter().chain(added).copied().max().unwrap_or(i64::MIN);
        // Old checkpoints above every change stay valid.
        let keep = match (added.is_empty(), gone.iter().min()) {
            (true, Some(&min_gone)) => old.partition_point(|c| c.pos < min_gone),
            _ => 0,
        };
        let mut out: Vec<Checkpoint> = old[..keep].to_vec();
        let (mut tracker, from) = match keep {
            0 => (Tracker::default(), i64::MIN),
            k => (Tracker::resume(&old[k - 1]), old[k - 1].pos),
        };

        let mut stmt =
            conn.prepare("SELECT pos, oid, parents FROM commits WHERE pos >= ?1 ORDER BY pos")?;
        let mut it = stmt.query([from])?;
        while let Some(r) = it.next()? {
            let pos: i64 = r.get(0)?;
            if pos > settled_after {
                if let Ok(i) = old.binary_search_by_key(&pos, |c| c.pos) {
                    if old[i].lanes == tracker.layout.lanes() {
                        out.extend_from_slice(&old[i..]);
                        return Ok(Some(out));
                    }
                }
            }
            tracker.feed(pos, blob(r.get_ref(1)?), blob(r.get_ref(2)?), &mut out);
        }
        Ok(Some(out))
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
        let quick_tips = new.clone();
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
                st.checkpoints.clear();
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
                let mut tracker = Tracker::default();
                // While git works out the full order, show a first page from
                // the newest tips (see `quick_prefix`); the full stream then
                // has to agree with it row by row.
                let mut prefix = match quick_prefix(repo, &quick_tips, cancel) {
                    Some(rows) => {
                        let mut first = rows.clone();
                        self.append(w, &mut first, &mut next, &mut tracker)?;
                        on_status(self.status());
                        rows
                    }
                    None => Vec::new(),
                };
                let mut verified = 0;
                let written = rx.iter().try_for_each(|mut rows| {
                    if verified < prefix.len() {
                        let same = rows
                            .iter()
                            .zip(&prefix[verified..])
                            .take_while(|(a, b)| a.oid == b.oid)
                            .count();
                        if verified + same == prefix.len() || same == rows.len() {
                            verified += same;
                            rows.drain(..same);
                        } else {
                            // The full order differs (clock skew): start over
                            // with what git says, the matching rows included.
                            tracing::info!(
                                "log index: quick first page differed at row {}",
                                verified + same
                            );
                            let mut redo: Vec<Record> = prefix[..verified].to_vec();
                            redo.append(&mut rows);
                            rows = redo;
                            prefix.clear();
                            self.reset_rows(w)?;
                            next = 0;
                            tracker = Tracker::default();
                        }
                    }
                    self.append(w, &mut rows, &mut next, &mut tracker)?;
                    on_status(self.status());
                    Ok::<_, IndexError>(())
                });
                if written.is_ok() && verified < prefix.len() {
                    // The full history is shorter than the quick page: keep what git said.
                    tracing::info!("log index: quick first page longer than history");
                    let mut redo: Vec<Record> = prefix[..verified].to_vec();
                    self.reset_rows(w)?;
                    next = 0;
                    tracker = Tracker::default();
                    self.append(w, &mut redo, &mut next, &mut tracker)?;
                }
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

    /// Drop every row written so far by the running full build.
    fn reset_rows(&self, w: &mut Writer) -> Result<()> {
        self.apply(
            w,
            |w| Ok(w.conn.execute_batch("DELETE FROM commits;")?),
            |st, ()| {
                st.order.clear();
                st.checkpoints.clear();
                st.version += 1;
            },
        )
    }

    fn append(
        &self,
        w: &mut Writer,
        batch: &mut Vec<Record>,
        next: &mut i64,
        tracker: &mut Tracker,
    ) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let first = *next;
        *next += batch.len() as i64;
        let mut checkpoints = Vec::new();
        for (i, r) in batch.iter().enumerate() {
            tracker.feed(first + i as i64, &r.oid, &r.parents, &mut checkpoints);
        }
        let rows = std::mem::take(batch);
        self.apply(
            w,
            |w| w.insert(rows, first),
            |st, keys| {
                st.order.extend(keys);
                st.checkpoints.extend(checkpoints);
            },
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
/// Tips beyond this many make a quick first page worthwhile.
const QUICK_MIN_TIPS: usize = 16;

/// The first rows of `git log --date-order` over all `tips`, computed from
/// the newest tips only.
///
/// With many tips spread through a long history (tags of every release),
/// git walks far down before it prints the first commit: 1.3 s on a million
/// commits with 800 refs, against 40 ms from HEAD alone. Date order puts a
/// commit after every newer one (barring clock skew), so tips older than the
/// oldest of the first rows cannot change them: leave those out, check that
/// they are indeed older, and add any that are not. `None` when it does not
/// apply (few tips, short history) or cannot be shown to be the same.
fn quick_prefix(repo: &Repo, tips: &RefTips, cancel: &CancelToken) -> Option<Vec<Record>> {
    if tips.tips.len() <= QUICK_MIN_TIPS {
        return None;
    }
    // Unknown dates (detached HEAD) count as newest: always included.
    let mut by_age: Vec<(&String, i64)> = tips
        .tips
        .iter()
        .map(|t| (t, *tips.times.get(t).unwrap_or(&i64::MAX)))
        .collect();
    by_age.sort_by_key(|&(_, t)| std::cmp::Reverse(t));
    let mut take = QUICK_MIN_TIPS;
    for _ in 0..3 {
        let chosen: Vec<String> = by_age[..take].iter().map(|(t, _)| (*t).clone()).collect();
        let limit = format!("--max-count={FIRST_BATCH}");
        let mut rows = Vec::new();
        let mut parser = RecordParser::default();
        repo.cli()
            .run_streaming(
                repo.root(),
                &["log", "--date-order", FORMAT, &limit, "--stdin"],
                RunOptions::read()
                    .stdin(rev_input(&chosen, &[]))
                    .cancel(cancel.clone()),
                |chunk| {
                    parser.feed(chunk, |r| rows.push(r));
                    true
                },
            )
            .ok()?;
        if rows.len() < FIRST_BATCH {
            return None;
        }
        let oldest = rows.iter().map(|r| r.commit_time).min()?;
        // Left-out tips at least as new as the oldest row could belong among
        // the first rows: take them too and try again.
        let newer = by_age[take..]
            .iter()
            .take_while(|(_, t)| *t >= oldest)
            .count();
        if newer == 0 {
            return Some(rows);
        }
        take += newer;
    }
    None
}

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
