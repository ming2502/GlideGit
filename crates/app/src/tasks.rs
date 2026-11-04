//! Long-running, cancellable operations started from the UI.
//!
//! The frontend picks a numeric task id, passes it to the command, listens
//! for `task-progress` events with that id and may call `cancel_task(id)`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glide_git_backend::{CancelToken, NetOptions, Progress, ProgressSink};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const PROGRESS_EVENT: &str = "task-progress";

/// Minimum spacing between two progress events with the same phase/percent class.
const THROTTLE: Duration = Duration::from_millis(80);

#[derive(Default)]
pub struct TaskRegistry {
    tokens: Mutex<HashMap<u64, CancelToken>>,
}

impl TaskRegistry {
    /// Register a task; it is unregistered when the returned guard drops.
    pub fn start(self: &Arc<Self>, id: u64) -> TaskGuard {
        let token = CancelToken::new();
        self.tokens.lock().unwrap().insert(id, token.clone());
        TaskGuard {
            id,
            token,
            registry: self.clone(),
        }
    }

    pub fn cancel(&self, id: u64) -> bool {
        match self.tokens.lock().unwrap().get(&id) {
            Some(t) => {
                t.cancel();
                true
            }
            None => false,
        }
    }
}

pub struct TaskGuard {
    id: u64,
    token: CancelToken,
    registry: Arc<TaskRegistry>,
}

impl TaskGuard {
    /// Cancellation + throttled progress events for a network operation.
    pub fn net_options(&self, app: &AppHandle) -> NetOptions {
        let app = app.clone();
        let id = self.id;
        let last: Mutex<Option<(Instant, String, Option<u8>)>> = Mutex::new(None);
        let sink = ProgressSink::new(move |p: &Progress| {
            let mut last = last.lock().unwrap();
            let due = match &*last {
                None => true,
                // Always report a new phase or completion; otherwise throttle.
                Some((t, phase, pct)) => {
                    *phase != p.phase
                        || (p.percent == Some(100) && *pct != Some(100))
                        || (*pct != p.percent && t.elapsed() >= THROTTLE)
                }
            };
            if due {
                *last = Some((Instant::now(), p.phase.clone(), p.percent));
                let _ = app.emit(
                    PROGRESS_EVENT,
                    ProgressEvent {
                        task_id: id,
                        progress: p,
                    },
                );
            }
        });
        NetOptions {
            cancel: Some(self.token.clone()),
            progress: Some(sink),
        }
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.registry.tokens.lock().unwrap().remove(&self.id);
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    task_id: u64,
    progress: &'a Progress,
}
