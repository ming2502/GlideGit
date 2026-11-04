//! Routes git/ssh credential prompts from the askpass server to the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use glide_core::askpass::{classify, AskHandler, AskRequest};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

pub const ASK_EVENT: &str = "askpass-request";
/// Sent when a prompt is no longer wanted (timed out / git gave up).
pub const DISMISS_EVENT: &str = "askpass-dismiss";

#[derive(Default)]
pub struct AskpassBridge {
    app: OnceLock<AppHandle>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>,
    next: AtomicU64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Dismiss {
    id: u64,
}

/// Removes the pending entry and tells the UI to close the dialog if the
/// request ends without an answer (e.g. the server-side timeout drops it).
struct PendingGuard<'a> {
    bridge: &'a AskpassBridge,
    app: AppHandle,
    id: u64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if self
            .bridge
            .pending
            .lock()
            .unwrap()
            .remove(&self.id)
            .is_some()
        {
            let _ = self.app.emit(DISMISS_EVENT, Dismiss { id: self.id });
        }
    }
}

impl AskpassBridge {
    pub fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    pub fn handler(self: &Arc<Self>) -> AskHandler {
        let me = self.clone();
        Arc::new(move |prompt| {
            let me = me.clone();
            Box::pin(async move { me.ask(prompt).await })
        })
    }

    async fn ask(&self, prompt: String) -> Option<String> {
        let app = self.app.get()?.clone();
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let (kind, target) = classify(&prompt);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _guard = PendingGuard {
            bridge: self,
            app: app.clone(),
            id,
        };

        // The prompt may come from a background fetch: bring the window forward.
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        let _ = app.emit(
            ASK_EVENT,
            AskRequest {
                id,
                kind,
                prompt,
                target,
            },
        );
        rx.await.ok().flatten()
    }

    /// Deliver the user's answer (`None` = cancelled). False if the prompt is gone.
    pub fn respond(&self, id: u64, answer: Option<String>) -> bool {
        match self.pending.lock().unwrap().remove(&id) {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }
}
