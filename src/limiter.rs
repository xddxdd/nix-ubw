use std::collections::HashMap;
use std::collections::VecDeque;

use log::{info, warn};
use nix::sys::ptrace;
use nix::unistd::Pid;

use crate::resources::{profile_for, ResourceProfile};

/// Per-PID record of claimed resources.
struct ActiveEntry {
    profile: ResourceProfile,
}

/// A paused process waiting for resources to free up.
struct PausedEntry {
    pid: Pid,
    profile: ResourceProfile,
}

/// Result of the on_exec call.
pub enum OnExecResult {
    /// Process is not throttled.
    NotThrottled,
    /// Process is throttled and has been admitted.
    Admitted,
    /// Process is throttled and has been paused.
    Paused,
}

/// Tracks resource consumption of rate-limited processes and pauses new ones
/// when the budget (CPU cores or memory) is exhausted.
pub struct Limiter {
    /// Resources held by currently running throttled processes.
    active: HashMap<Pid, ActiveEntry>,
    /// Queue of processes waiting for resources.
    paused: VecDeque<PausedEntry>,
    /// Currently available (free) resources.
    free: ResourceProfile,
}

impl Limiter {
    pub fn new(total: ResourceProfile) -> Self {
        Self {
            active: HashMap::new(),
            paused: VecDeque::new(),
            free: total,
        }
    }

    /// Called on exec of a process. If the process is throttled, it is either
    /// admitted (returns Admitted) or paused (returns Paused). If the process
    /// is not throttled, it returns NotThrottled.
    ///
    /// The resource profile is calculated here and persisted for the lifecycle
    /// of the process in the limiter.
    pub fn on_exec(&mut self, pid: Pid, args: &[String]) -> OnExecResult {
        if let Some(profile) = profile_for(args) {
            if self.fits(&profile) {
                self.admit(pid, profile);
                OnExecResult::Admitted
            } else {
                info!(
                    "[limit] PID {} PAUSED — need {}, have {} free ({} paused)",
                    pid, profile, self.free, self.paused.len() + 1,
                );
                self.paused.push_back(PausedEntry { pid, profile });
                OnExecResult::Paused
            }
        } else {
            OnExecResult::NotThrottled
        }
    }

    /// Called when any process exits. If it was throttled, free its resources
    /// and try to resume waiting processes.
    pub fn on_exit(&mut self, pid: Pid) {
        if let Some(entry) = self.active.remove(&pid) {
            self.free += entry.profile;
            info!(
                "[limit] PID {} finished — {} free ({} paused)",
                pid, self.free, self.paused.len(),
            );
            self.try_resume_paused();
        }
        // Remove from paused too in case it exited before being resumed.
        self.paused.retain(|e| e.pid != pid);
    }

    /// Whether the given profile fits within remaining resources.
    fn fits(&self, profile: &ResourceProfile) -> bool {
        profile.has_free_resources(&self.free)
    }

    fn admit(&mut self, pid: Pid, profile: ResourceProfile) {
        self.free -= profile;
        self.active.insert(pid, ActiveEntry { profile });
        info!(
            "[limit] PID {} admitted — {} free ({} paused)",
            pid, self.free, self.paused.len(),
        );
    }

    fn try_resume_paused(&mut self) {
        // Walk the queue front-to-back; stop at the first entry that doesn't
        // fit (FIFO order preserved).
        while let Some(front) = self.paused.front() {
            if !self.fits(&front.profile) {
                break;
            }
            let entry = self.paused.pop_front().unwrap();
            info!(
                "[limit] Resuming PID {} — need {}",
                entry.pid, entry.profile,
            );
            let pid = entry.pid;
            self.admit(pid, entry.profile);
            if let Err(e) = ptrace::cont(pid, None) {
                warn!("Failed to resume paused PID {}: {}", pid, e);
                if let Some(entry) = self.active.remove(&pid) {
                    self.free += entry.profile;
                }
            }
        }
    }
}
