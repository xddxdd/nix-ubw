use std::collections::HashSet;
use std::fs;

use log::{debug, info, warn};
use nix::libc;
use nix::sys::ptrace;
use nix::sys::signal::Signal;
use nix::sys::wait::WaitStatus;
use nix::unistd::Pid;

use crate::limiter::Limiter;

/// All state for the tracer.
pub struct Tracer {
    /// All PIDs we are currently tracing.
    pub traced: HashSet<Pid>,
    /// Concurrency limiter for rate-limited processes.
    pub limiter: Limiter,
}

impl Tracer {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            traced: HashSet::new(),
            limiter: Limiter::new(max_concurrent),
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
                info!("[exit] PID {} exited with code {}", pid, code);
                self.traced.remove(&pid);
                self.limiter.on_exit(pid);
            }
            WaitStatus::Signaled(pid, sig, _core) => {
                info!("[exit] PID {} killed by {:?}", pid, sig);
                self.traced.remove(&pid);
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
                        let cmdline = read_cmdline(child_pid)
                            .map(|args| shell_join(&args))
                            .unwrap_or_else(|| "<unavailable>".into());
                        info!("[{}] PID {} -> PID {}: {}", event_name, pid, child_pid, cmdline);
                        self.traced.insert(child_pid);
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
                let args = read_cmdline(pid);
                let cmdline = args
                    .as_ref()
                    .map(|a| shell_join(a))
                    .unwrap_or_else(|| "<unavailable>".into());

                if args.as_ref().map_or(false, |a| Limiter::is_rate_limited(a)) {
                    let allowed = self.limiter.on_exec(pid);
                    if allowed {
                        info!(
                            "[exec] PID {}: {} ({} active, {} paused)",
                            pid, cmdline,
                            self.limiter.active_count(),
                            self.limiter.paused_count()
                        );
                        if let Err(e) = ptrace::cont(pid, None) {
                            warn!("Failed to continue {} after exec: {}", pid, e);
                        }
                    } else {
                        info!(
                            "[exec] PID {}: {} -- PAUSED ({} active, {} paused)",
                            pid, cmdline,
                            self.limiter.active_count(),
                            self.limiter.paused_count()
                        );
                    }
                } else {
                    info!("[exec] PID {}: {}", pid, cmdline);
                    if let Err(e) = ptrace::cont(pid, None) {
                        warn!("Failed to continue {} after exec: {}", pid, e);
                    }
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

/// Read /proc/<pid>/cmdline and return the arguments as a Vec<String>.
pub fn read_cmdline(pid: Pid) -> Option<Vec<String>> {
    let path = format!("/proc/{}/cmdline", pid);
    let data = fs::read(&path).ok()?;
    let args: Vec<String> = data
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Some(args)
}

/// Join args into a shell-like representation for logging.
fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(' ') || a.contains('\'') || a.contains('"') || a.is_empty() {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
