use std::collections::HashMap;
use std::collections::VecDeque;

use log::{info, warn};
use nix::sys::ptrace;
use nix::unistd::Pid;

use crate::resources::{profile_for, ResourceProfile};

/// Per-PID record of claimed resources.
struct ActiveEntry {
    name: String,
    profile: ResourceProfile,
}

/// A paused process waiting for resources to free up.
struct PausedEntry {
    pid: Pid,
    name: String,
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
    /// Total resource budget.
    total: ResourceProfile,
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
            total,
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
        if let Some(profile) = profile_for(args, &self.total) {
            let name = args
                .first()
                .cloned()
                .unwrap_or_else(|| "<unavailable>".into());
            if self.fits(&profile) {
                self.admit(pid, name, profile);
                OnExecResult::Admitted
            } else {
                info!(
                    "[limit] {} ({}) PAUSED - need {}, free: {}, total: {} ({} paused)",
                    name,
                    pid,
                    profile,
                    self.free,
                    self.total,
                    self.paused.len() + 1,
                );
                self.paused.push_back(PausedEntry { pid, name, profile });
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
                "[limit] {} ({}) finished - free: {}, total: {} ({} paused)",
                entry.name,
                pid,
                self.free,
                self.total,
                self.paused.len(),
            );
            self.try_resume_paused();
        }
        // Remove from paused too in case it exited before being resumed.
        self.paused.retain(|e| e.pid != pid);
    }

    /// Whether the given profile fits within remaining resources.
    /// Failsafe: if nothing else is active, it always fits (deadlock prevention).
    fn fits(&self, profile: &ResourceProfile) -> bool {
        if profile.has_free_resources(&self.free) {
            true
        } else if self.active.is_empty() {
            warn!(
                "[limit] Budget exceeded but no active tasks, force admitting process needing {}",
                profile
            );
            true
        } else {
            false
        }
    }

    fn admit(&mut self, pid: Pid, name: String, profile: ResourceProfile) {
        self.free -= profile;
        info!(
            "[limit] {} ({}) admitted - free: {}, total: {} ({} paused)",
            name,
            pid,
            self.free,
            self.total,
            self.paused.len(),
        );
        self.active.insert(pid, ActiveEntry { name, profile });
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
                "[limit] Resuming {} ({}) - need {}",
                entry.name, entry.pid, entry.profile,
            );
            let pid = entry.pid;
            self.admit(pid, entry.name, entry.profile);
            if let Err(e) = ptrace::cont(pid, None) {
                warn!("Failed to resume paused PID {}: {}", pid, e);
                if let Some(entry) = self.active.remove(&pid) {
                    self.free += entry.profile;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::Pid;

    #[test]
    fn test_not_throttled() {
        let mut limiter = Limiter::new(ResourceProfile::new(2, 2));
        let res = limiter.on_exec(Pid::from_raw(100), &["some_random_process".into()]);
        assert!(matches!(res, OnExecResult::NotThrottled));
        assert!(limiter.active.is_empty());
        assert!(limiter.paused.is_empty());
        assert_eq!(limiter.free, ResourceProfile::new(2, 2));
    }

    #[test]
    fn test_admit_and_pause() {
        let mut limiter = Limiter::new(ResourceProfile::new(2, 2));

        // cc needs (1, 1). Normally fits.
        let res1 = limiter.on_exec(Pid::from_raw(100), &["cc".into()]);
        assert!(matches!(res1, OnExecResult::Admitted));
        assert_eq!(limiter.active.len(), 1);
        assert_eq!(limiter.free, ResourceProfile::new(1, 1));

        // another cc fits.
        let res2 = limiter.on_exec(Pid::from_raw(101), &["cc".into()]);
        assert!(matches!(res2, OnExecResult::Admitted));
        assert_eq!(limiter.active.len(), 2);
        assert_eq!(limiter.free, ResourceProfile::new(0, 0));

        // third cc pauses.
        let res3 = limiter.on_exec(Pid::from_raw(102), &["cc".into()]);
        assert!(matches!(res3, OnExecResult::Paused));
        assert_eq!(limiter.active.len(), 2);
        assert_eq!(limiter.paused.len(), 1);
        assert_eq!(limiter.free, ResourceProfile::new(0, 0));
    }

    #[test]
    fn test_force_admit() {
        let mut limiter = Limiter::new(ResourceProfile::new(1, 1));

        // rustc needs (1, 4). > (1, 1).
        // normally it would be paused, but since active is empty, it force admits.
        let res1 = limiter.on_exec(Pid::from_raw(100), &["rustc".into()]);
        assert!(matches!(res1, OnExecResult::Admitted));
        assert_eq!(limiter.active.len(), 1);
        assert_eq!(limiter.free, ResourceProfile::new(0, -3));

        // a second rustc should pause because active is no longer empty.
        let res2 = limiter.on_exec(Pid::from_raw(101), &["rustc".into()]);
        assert!(matches!(res2, OnExecResult::Paused));
        assert_eq!(limiter.active.len(), 1);
        assert_eq!(limiter.paused.len(), 1);
        assert_eq!(limiter.free, ResourceProfile::new(0, -3));

        limiter.on_exit(Pid::from_raw(100));

        // PID 100 exits, so its resources (1, 4) are freed, making free (1, 1).
        // It pops PID 101 to force admit, but ptrace::cont fails in unit tests,
        // so it cleans up PID 101 from active and frees its resources as well.
        assert_eq!(limiter.active.len(), 0);
        assert_eq!(limiter.paused.len(), 0);
        assert_eq!(limiter.free, ResourceProfile::new(1, 1));
    }

    #[test]
    fn test_on_exit() {
        let mut limiter = Limiter::new(ResourceProfile::new(2, 2));

        limiter.on_exec(Pid::from_raw(100), &["cc".into()]); // admits, free (1, 1)
        limiter.on_exec(Pid::from_raw(101), &["cc".into()]); // admits, free (0, 0)
        limiter.on_exec(Pid::from_raw(102), &["cc".into()]); // pauses
        limiter.on_exec(Pid::from_raw(103), &["cc".into()]); // pauses

        assert_eq!(limiter.active.len(), 2);
        assert_eq!(limiter.paused.len(), 2);
        assert_eq!(limiter.free, ResourceProfile::new(0, 0));

        limiter.on_exit(Pid::from_raw(100));

        // Since 100 exits, free becomes (1, 1).
        // try_resume_paused pops 102 and calls ptrace::cont which fails (no such process).
        // It's then removed from active, making free (1, 1) again.
        // Then it pops 103, same thing happens.
        // Finally paused is empty and active only has 101.
        assert_eq!(limiter.active.len(), 1);
        assert_eq!(limiter.paused.len(), 0);
        assert_eq!(limiter.free, ResourceProfile::new(1, 1));
    }
}
