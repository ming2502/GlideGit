//! Commit graph layout: which lane (column) each commit sits in and which
//! lines pass through each row.
//!
//! Rows are processed top to bottom while carrying the *lane state*: the
//! ordered list of lanes open at the boundary between two rows, each waiting
//! for a particular commit. For a row's commit:
//!
//! - Lanes waiting for it converge on its node; the leftmost one is its lane.
//!   A commit nobody waits for (a branch tip) opens a new lane on the right.
//! - Its first parent continues in its lane. Each further parent joins a lane
//!   already waiting for it, or opens a new lane right after the node.
//! - Lanes that ended are removed and the rest close up to the left.
//!
//! Each row is drawn as two halves so neighbouring rows join seamlessly:
//! the *upper* half runs from the boundary above to the row's centre, the
//! *lower* half from the centre to the boundary below. Lanes never move in
//! the upper half (so nothing crosses a node); all sideways moves happen in
//! the lower half.
//!
//! The state depends only on the commits, never on where the walk started,
//! so a layout can resume from any saved state ([`Layout::from_lanes`]).
//! That lets the index keep a checkpoint every few hundred rows and lay out
//! just the rows on screen.
//!
//! Colours are not part of the layout: the UI colours by column. A colour
//! carried down from the lane's tip would change all the way down a branch
//! whenever a commit is added on top, and saved states would never match
//! again after an update.

use serde::Serialize;
use ts_rs::TS;

/// A lane at a row boundary: the commit it is waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub oid: Box<[u8]>,
}

/// Drawing data for one row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GraphRow {
    /// Column of the commit's node.
    pub lane: u16,
    /// Upper-half lines as flat `[from, to]` pairs: `from` is the column at
    /// the top edge, `to` the column at the centre. A line ending on `lane`
    /// enters the node; any other line passes straight through (`from == to`).
    pub up: Vec<u16>,
    /// Lower-half lines as flat `[from, to]` pairs: `from` is the column at
    /// the centre, `to` the column at the bottom edge. Lines from `lane` lead
    /// to the commit's parents.
    pub down: Vec<u16>,
}

impl GraphRow {
    /// Columns this row needs.
    pub fn width(&self) -> u16 {
        self.up
            .iter()
            .chain(&self.down)
            .chain([&self.lane])
            .max()
            .copied()
            .unwrap_or(0)
            + 1
    }
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    lanes: Vec<Lane>,
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_lanes(lanes: Vec<Lane>) -> Self {
        Self { lanes }
    }

    /// The state at the current boundary (before the next row).
    pub fn lanes(&self) -> &[Lane] {
        &self.lanes
    }

    /// Lay out the next row. `parents` holds the raw parent ids concatenated
    /// (each `oid.len()` bytes). With `out`, the row's drawing data is written
    /// there; without it only the state advances.
    pub fn step(&mut self, oid: &[u8], parents: &[u8], out: Option<&mut GraphRow>) {
        let n = oid.len().max(1);
        let parents: Vec<&[u8]> = parents.chunks(n).collect();
        let old = &self.lanes;
        let ends_here = |l: &Lane| *l.oid == *oid;

        let node = old.iter().position(ends_here).unwrap_or(old.len());
        // Further parents that a passing lane already waits for are joined
        // there rather than getting a lane of their own.
        let waited_for = |p: &[u8]| old.iter().any(|l| !ends_here(l) && *l.oid == *p);

        let mut new: Vec<Lane> = Vec::with_capacity(old.len() + parents.len());
        // New column of each old lane that passes through (None: ends here).
        let mut moved: Vec<Option<usize>> = vec![None; old.len()];
        // Columns of the lanes the node opens.
        let mut opened: Vec<usize> = Vec::new();
        let mut joins: Vec<&[u8]> = Vec::new();

        let mut open_node_lanes = |new: &mut Vec<Lane>, opened: &mut Vec<usize>| {
            for (k, p) in parents.iter().enumerate() {
                // The first parent always continues in the node's lane.
                if k > 0 && waited_for(p) {
                    joins.push(p);
                } else {
                    opened.push(new.len());
                    new.push(Lane { oid: (*p).into() });
                }
            }
        };

        for (i, lane) in old.iter().enumerate() {
            if i == node {
                open_node_lanes(&mut new, &mut opened);
            } else if !ends_here(lane) {
                moved[i] = Some(new.len());
                new.push(lane.clone());
            }
        }
        if node == old.len() {
            open_node_lanes(&mut new, &mut opened);
        }

        if let Some(out) = out {
            out.lane = node as u16;
            out.up.clear();
            out.down.clear();
            for (i, lane) in old.iter().enumerate() {
                let to = if ends_here(lane) { node } else { i };
                out.up.extend([i as u16, to as u16]);
                if let Some(j) = moved[i] {
                    out.down.extend([i as u16, j as u16]);
                }
            }
            for j in &opened {
                out.down.extend([node as u16, *j as u16]);
            }
            for p in &joins {
                // The leftmost passing lane waiting for this parent.
                let target = (0..old.len()).find_map(|i| moved[i].filter(|_| *old[i].oid == **p));
                if let Some(j) = target {
                    out.down.extend([node as u16, j as u16]);
                }
            }
        }

        self.lanes = new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn id(i: usize) -> Vec<u8> {
        // Two bytes are enough for tests.
        vec![(i % 251) as u8, (i / 251) as u8]
    }

    fn layout(commits: &[(usize, Vec<usize>)]) -> Vec<GraphRow> {
        let mut l = Layout::new();
        commits
            .iter()
            .map(|(c, ps)| {
                let parents: Vec<u8> = ps.iter().flat_map(|p| id(*p)).collect();
                let mut row = GraphRow::default();
                l.step(&id(*c), &parents, Some(&mut row));
                row
            })
            .collect()
    }

    fn pairs(v: &[u16]) -> Vec<(u16, u16)> {
        v.chunks(2).map(|t| (t[0], t[1])).collect()
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = layout(&[(3, vec![2]), (2, vec![1]), (1, vec![])]);
        assert!(rows.iter().all(|r| r.lane == 0 && r.width() == 1));
        assert_eq!(pairs(&rows[0].up), vec![]);
        assert_eq!(pairs(&rows[0].down), vec![(0, 0)]);
        assert_eq!(pairs(&rows[1].up), vec![(0, 0)]);
        assert_eq!(pairs(&rows[2].down), vec![]);
    }

    #[test]
    fn branch_and_merge() {
        // 6 merges 5 (main) and 4 (feature); both fork from 2.
        //   6
        //   |\
        //   5 |
        //   | 4
        //   | 3
        //   |/
        //   2
        //   1
        let rows = layout(&[
            (6, vec![5, 4]),
            (5, vec![2]),
            (4, vec![3]),
            (3, vec![2]),
            (2, vec![1]),
            (1, vec![]),
        ]);
        let lanes: Vec<u16> = rows.iter().map(|r| r.lane).collect();
        assert_eq!(lanes, vec![0, 0, 1, 1, 0, 0]);
        assert_eq!(pairs(&rows[0].down), vec![(0, 0), (0, 1)]);
        // The feature lane converges on 2 from column 1.
        assert_eq!(pairs(&rows[4].up), vec![(0, 0), (1, 0)]);
        assert_eq!(pairs(&rows[4].down), vec![(0, 0)]);
    }

    #[test]
    fn tips_open_lanes_on_the_right_and_lanes_close_up() {
        // Two tips: 5 → 2 → 1 in lane 0, and 4 → 3 (a root) in lane 1.
        let rows = layout(&[
            (5, vec![2]),
            (4, vec![3]),
            (3, vec![]),
            (2, vec![1]),
            (1, vec![]),
        ]);
        assert_eq!(
            rows.iter().map(|r| r.lane).collect::<Vec<_>>(),
            vec![0, 1, 1, 0, 0]
        );
        assert_eq!(pairs(&rows[1].up), vec![(0, 0)]);
        assert_eq!(pairs(&rows[2].down), vec![(0, 0)]);

        // Lane on the left ends: the lane to its right moves over in the
        // lower half of that row.
        let rows = layout(&[(5, vec![3]), (4, vec![2]), (3, vec![]), (2, vec![])]);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(pairs(&rows[2].up), vec![(0, 0), (1, 1)]);
        assert_eq!(pairs(&rows[2].down), vec![(1, 0)]);
        assert_eq!(rows[3].lane, 0);
    }

    #[test]
    fn merge_joins_a_lane_already_waiting() {
        // 4 and 5 are tips; 5 merges 3, which 4's lane already waits for.
        let rows = layout(&[(4, vec![3]), (5, vec![2, 3]), (3, vec![2]), (2, vec![])]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(pairs(&rows[1].down), vec![(0, 0), (1, 1), (1, 0)]);
        assert_eq!(rows[1].width(), 2);
    }

    /// Follow the lines from each node and check they reach exactly its parents.
    fn check_connectivity(commits: &[(usize, Vec<usize>)], rows: &[GraphRow]) {
        let row_of: HashMap<usize, usize> = commits
            .iter()
            .enumerate()
            .map(|(r, (c, _))| (*c, r))
            .collect();
        for (r, (_, parents)) in commits.iter().enumerate() {
            let mut reached = HashSet::new();
            for t in rows[r].down.chunks(2).filter(|t| t[0] == rows[r].lane) {
                let mut col = t[1];
                let mut k = r + 1;
                loop {
                    let up = rows[k]
                        .up
                        .chunks(2)
                        .find(|u| u[0] == col)
                        .unwrap_or_else(|| panic!("row {k}: no line continues from column {col}"));
                    // Pass-through lines never end on the node's column.
                    if up[1] == rows[k].lane {
                        reached.insert(k);
                        break;
                    }
                    let down = rows[k]
                        .down
                        .chunks(2)
                        .find(|d| d[0] == up[1])
                        .unwrap_or_else(|| panic!("row {k}: line in column {} stops", up[1]));
                    col = down[1];
                    k += 1;
                }
            }
            let want: HashSet<usize> = parents.iter().map(|p| row_of[p]).collect();
            assert_eq!(reached, want, "parents of row {r}");
        }
        // Every line leaving a row's bottom edge enters the next row.
        for r in 0..rows.len() - 1 {
            let bottom: HashSet<u16> = rows[r].down.chunks(2).map(|t| t[1]).collect();
            let top: HashSet<u16> = rows[r + 1].up.chunks(2).map(|t| t[0]).collect();
            assert_eq!(bottom, top, "boundary below row {r}");
        }
    }

    /// Small deterministic PRNG so the test needs no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    #[test]
    fn random_histories_are_connected() {
        for seed in 1..200u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let n = 5 + rng.below(60);
            // Row r's parents are later rows, so the order is topological.
            let commits: Vec<(usize, Vec<usize>)> = (0..n)
                .map(|r| {
                    let later = n - r - 1;
                    let k = if later == 0 {
                        0
                    } else {
                        match rng.below(10) {
                            0 => 0,
                            1 | 2 => 2.min(later),
                            3 => 3.min(later),
                            _ => 1,
                        }
                    };
                    let mut ps: Vec<usize> = Vec::new();
                    while ps.len() < k {
                        let p = r + 1 + rng.below(later.min(8));
                        if !ps.contains(&p) {
                            ps.push(p);
                        }
                    }
                    (r, ps)
                })
                .collect();
            let rows = layout(&commits);
            check_connectivity(&commits, &rows);
        }
    }

    #[test]
    fn resuming_from_a_saved_state_gives_the_same_rows() {
        let commits: Vec<(usize, Vec<usize>)> = vec![
            (9, vec![7, 8]),
            (8, vec![6]),
            (7, vec![5]),
            (6, vec![4]),
            (5, vec![4, 3]),
            (4, vec![2]),
            (3, vec![2]),
            (2, vec![1]),
            (1, vec![]),
        ];
        let full = layout(&commits);
        let mut l = Layout::new();
        for (c, ps) in &commits[..4] {
            let parents: Vec<u8> = ps.iter().flat_map(|p| id(*p)).collect();
            l.step(&id(*c), &parents, None);
        }
        let mut resumed = Layout::from_lanes(l.lanes().to_vec());
        for (r, (c, ps)) in commits.iter().enumerate().skip(4) {
            let parents: Vec<u8> = ps.iter().flat_map(|p| id(*p)).collect();
            let mut row = GraphRow::default();
            resumed.step(&id(*c), &parents, Some(&mut row));
            assert_eq!(row, full[r]);
        }
    }
}
