use std::collections::{HashSet, VecDeque};

use log::{info, warn};
use nix::sys::ptrace;
use nix::unistd::Pid;



/// Names of executables to rate-limit (matched against basename of argv[0]).
const RATE_LIMITED: &[&str] = &["sleep"];

/// State for concurrency control of rate-limited processes.
pub struct Limiter {
    /// Maximum number of concurrently running rate-limited processes.
    max_concurrent: usize,
    /// PIDs of rate-limited processes that are actively running.
    active: HashSet<Pid>,
    /// PIDs of rate-limited processes that are paused (waiting for a slot).
    paused: VecDeque<Pid>,
}

impl Limiter {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            active: HashSet::new(),
            paused: VecDeque::new(),
        }
    }

    /// Check if a command should be rate-limited based on its argv[0].
    /// Assumes argv[0] has already been resolved by `read_cmdline`.
    pub fn is_rate_limited(args: &[String]) -> bool {
        if let Some(arg0) = args.first() {
            RATE_LIMITED.iter().any(|&name| arg0 == name)
        } else {
            false
        }
    }

    /// Called on exec of a rate-limited process. Returns true if the process
    /// should be continued, false if it should stay paused.
    pub fn on_exec(&mut self, pid: Pid) -> bool {
        if self.active.len() < self.max_concurrent {
            self.active.insert(pid);
            true
        } else {
            self.paused.push_back(pid);
            false
        }
    }

    /// Called when a process exits. If it was rate-limited, try to resume a paused one.
    pub fn on_exit(&mut self, pid: Pid) {
        if self.active.remove(&pid) {
            info!(
                "[limit] PID {} finished ({} active, {} paused)",
                pid,
                self.active.len(),
                self.paused.len()
            );
            self.try_resume_paused();
        }
        // Also remove from paused in case it exited before being resumed.
        self.paused.retain(|&p| p != pid);
    }

    /// Number of active rate-limited processes.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Number of paused rate-limited processes.
    pub fn paused_count(&self) -> usize {
        self.paused.len()
    }

    fn try_resume_paused(&mut self) {
        while self.active.len() < self.max_concurrent {
            if let Some(pid) = self.paused.pop_front() {
                info!(
                    "[limit] Resuming paused PID {} ({} active, {} paused)",
                    pid,
                    self.active.len() + 1,
                    self.paused.len()
                );
                self.active.insert(pid);
                if let Err(e) = ptrace::cont(pid, None) {
                    warn!("Failed to resume paused PID {}: {}", pid, e);
                    self.active.remove(&pid);
                }
            } else {
                break;
            }
        }
    }
}
