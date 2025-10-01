//! Pause + steering coordination primitives.
//!
//! A single [`PauseController`] is shared (Arc) between the TUI, every
//! agent's streaming loop, the pipeline orchestrator, and each tool
//! call guard. It provides two capabilities:
//!
//! 1. **Pause / resume.** `await_if_paused().await` yields an immediate
//!    future when running, or blocks until the controller is resumed.
//!    The runtime (Docker container) deliberately does not consult it —
//!    only the Interpreter, Architect, Reflector, pipeline phase
//!    transitions, and tool-call bookkeeping are pausable.
//!
//! 2. **Steering.** While paused, the TUI can push a steering message
//!    targeted at a specific [`Agent`]. The streaming helper polls its
//!    agent's queue and, when a steering message arrives, aborts the
//!    current `stream_prompt` (dropping the stream future). The caller
//!    then issues a fresh prompt that includes the steering text.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tokio::sync::Notify;

use crate::events::Agent;

/// Shared pause + steering coordination.
#[derive(Debug)]
pub struct PauseController {
    paused: AtomicBool,
    notify: Notify,
    steer: Mutex<HashMap<Agent, Vec<String>>>,
    steer_notify: Notify,
}

impl Default for PauseController {
    fn default() -> Self {
        Self::new()
    }
}

impl PauseController {
    pub fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            notify: Notify::new(),
            steer: Mutex::new(HashMap::new()),
            steer_notify: Notify::new(),
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Flip the paused flag. Returns the new value. Waking everyone
    /// who is currently parked in [`await_if_paused`] happens on the
    /// `false` edge.
    pub fn set_paused(&self, v: bool) -> bool {
        self.paused.store(v, Ordering::SeqCst);
        if !v {
            self.notify.notify_waiters();
        }
        v
    }

    /// Toggle and return the new value.
    pub fn toggle(&self) -> bool {
        let new = !self.is_paused();
        self.set_paused(new);
        new
    }

    /// Yields an already-ready future when not paused. Otherwise waits
    /// for a resume notification and returns.
    pub async fn await_if_paused(&self) {
        while self.is_paused() {
            // Register waiter BEFORE re-checking to avoid the lost-wakeup race.
            let waiter = self.notify.notified();
            if !self.is_paused() {
                break;
            }
            waiter.await;
        }
    }

    /// Enqueue a steering message for `agent`. Wakes any streaming loop
    /// that is currently selecting on `steer_notified`.
    pub fn push_steer(&self, agent: Agent, message: String) {
        let mut g = self.steer.lock().expect("steer mutex poisoned");
        g.entry(agent).or_default().push(message);
        self.steer_notify.notify_waiters();
    }

    /// Drain any steering messages queued for `agent`. Returns empty
    /// when nothing is pending.
    pub fn drain_steer(&self, agent: Agent) -> Vec<String> {
        let mut g = self.steer.lock().expect("steer mutex poisoned");
        g.remove(&agent).unwrap_or_default()
    }

    /// True if at least one steering message is queued for `agent`.
    pub fn has_steer(&self, agent: Agent) -> bool {
        let g = self.steer.lock().expect("steer mutex poisoned");
        g.get(&agent).map(|v| !v.is_empty()).unwrap_or(false)
    }

    /// Resolves the next time any agent gets a steering message.
    pub async fn steer_notified(&self) {
        self.steer_notify.notified().await
    }
}
