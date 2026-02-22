use log::{debug, info, warn};
use nix::libc;
use nix::sys::ptrace;
use nix::sys::signal::Signal;
use nix::sys::wait::WaitStatus;
use nix::unistd::Pid;

use crate::limiter::Limiter;
use crate::nixutil;
use crate::resources::ResourceProfile;

/// All state for the tracer.
pub struct Tracer {
    /// Concurrency limiter for rate-limited processes.
    pub limiter: Limiter,
}

impl Tracer {
    pub fn new(total: ResourceProfile) -> Self {
        Self {
            limiter: Limiter::new(total),
        }
    }

    pub fn handle_wait_status(&mut self, status: WaitStatus) {
        match status {
            WaitStatus::PtraceEvent(pid, _sig, event) => {
                self.handle_ptrace_event(pid, event);
            }
            WaitStatus::Stopped(pid, sig) => {
                let forward = if sig == Signal::SIGTRAP || sig == Signal::SIGSTOP {
                    None
                } else {
                    Some(sig)
                };
                debug!("PID {} stopped by {:?}, forwarding={:?}", pid, sig, forward);
                if let Err(e) = ptrace::cont(pid, forward) {
                    warn!("Failed to continue {} after {:?}: {}", pid, sig, e);
                }
            }
            WaitStatus::Exited(pid, code) => {
                debug!("[exit] PID {} exited with code {}", pid, code);
                self.limiter.on_exit(pid);
            }
            WaitStatus::Signaled(pid, sig, _core) => {
                debug!("[exit] PID {} killed by {:?}", pid, sig);
                self.limiter.on_exit(pid);
            }
            other => {
                debug!("PID {:?}: {:?}", other.pid(), other);
                if let Some(pid) = other.pid() {
                    let _ = ptrace::cont(pid, None);
                }
            }
        }
    }

    fn handle_ptrace_event(&mut self, pid: Pid, event: i32) {
        match event {
            libc::PTRACE_EVENT_FORK | libc::PTRACE_EVENT_VFORK | libc::PTRACE_EVENT_CLONE => {
                match ptrace::getevent(pid) {
                    Ok(child_pid_raw) => {
                        let child_pid = Pid::from_raw(child_pid_raw as i32);
                        let event_name = match event {
                            libc::PTRACE_EVENT_FORK => "fork",
                            libc::PTRACE_EVENT_VFORK => "vfork",
                            libc::PTRACE_EVENT_CLONE => "clone",
                            _ => unreachable!(),
                        };
                        let basename = nixutil::read_cmdline(child_pid)
                            .and_then(|a| a.into_iter().next())
                            .unwrap_or_else(|| "<unavailable>".into());
                        info!("[{}] PID {} -> PID {}: {}", event_name, pid, child_pid, basename);
                    }
                    Err(e) => {
                        warn!("Failed to get child PID from {}: {}", pid, e);
                    }
                }
                if let Err(e) = ptrace::cont(pid, None) {
                    warn!("Failed to continue {} after fork: {}", pid, e);
                }
            }
            libc::PTRACE_EVENT_EXEC => {
                let args = nixutil::read_cmdline(pid);
                let basename = args
                    .as_ref()
                    .and_then(|a| a.first())
                    .map(|a| a.as_str())
                    .unwrap_or("<unavailable>");

                if let Some(ref a) = args {
                    match self.limiter.on_exec(pid, a) {
                        crate::limiter::OnExecResult::Admitted => {
                            info!("[exec] PID {}: {} (admitted)", pid, basename);
                            if let Err(e) = ptrace::cont(pid, None) {
                                warn!("Failed to continue {} after exec: {}", pid, e);
                                self.limiter.on_exit(pid);
                            }
                            return;
                        }
                        crate::limiter::OnExecResult::Paused => {
                            info!("[exec] PID {}: {} (paused)", pid, basename);
                            // Do not call ptrace::cont — process stays stopped.
                            return;
                        }
                        crate::limiter::OnExecResult::NotThrottled => {}
                    }
                }
                info!("[exec] PID {}: {}", pid, basename);
                if let Err(e) = ptrace::cont(pid, None) {
                    warn!("Failed to continue {} after exec: {}", pid, e);
                }
            }
            libc::PTRACE_EVENT_STOP => {
                debug!("PID {} PTRACE_EVENT_STOP", pid);
                if let Err(e) = ptrace::cont(pid, None) {
                    warn!("Failed to continue {} after stop: {}", pid, e);
                }
            }
            _ => {
                warn!("PID {} unknown event {}", pid, event);
                let _ = ptrace::cont(pid, None);
            }
        }
    }
}

