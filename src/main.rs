use std::collections::HashSet;
use std::fs;

use anyhow::{bail, Context, Result};
use log::{debug, error, info, warn};
use nix::libc;
use nix::sys::ptrace;
use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

/// Scan /proc for all processes whose cmdline is "nix-daemon --daemon".
fn find_nix_daemon_pids() -> Result<Vec<Pid>> {
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc").context("Failed to read /proc")? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let pid: i32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let pid = Pid::from_raw(pid);
        if let Some(args) = read_cmdline(pid) {
            if args.len() >= 2
                && args[0].ends_with("nix-daemon")
                && args[1] == "--daemon"
            {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

/// Read /proc/<pid>/cmdline and return the arguments as a Vec<String>.
fn read_cmdline(pid: Pid) -> Option<Vec<String>> {
    let path = format!("/proc/{}/cmdline", pid);
    let data = fs::read(&path).ok()?;
    let args: Vec<String> = data
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Some(args)
}

/// The ptrace options we set on every tracee.
fn trace_options() -> ptrace::Options {
    ptrace::Options::PTRACE_O_TRACEFORK
        | ptrace::Options::PTRACE_O_TRACEVFORK
        | ptrace::Options::PTRACE_O_TRACECLONE
        | ptrace::Options::PTRACE_O_TRACEEXEC
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let daemon_pids = find_nix_daemon_pids()?;
    if daemon_pids.is_empty() {
        bail!("No nix-daemon processes found (looking for cmdline 'nix-daemon --daemon')");
    }

    let mut traced: HashSet<Pid> = HashSet::new();

    for &pid in &daemon_pids {
        match ptrace::seize(pid, trace_options()) {
            Ok(()) => {
                info!("Attached to nix-daemon (pid {})", pid);
                traced.insert(pid);
            }
            Err(e) => {
                warn!("Failed to attach to pid {}: {} (are you root?)", pid, e);
            }
        }
    }

    if traced.is_empty() {
        bail!("Failed to attach to any nix-daemon process");
    }

    // Default SIGINT/SIGTERM handler will kill this process.
    // ptrace automatically detaches all tracees when the tracer exits.
    info!("Tracing started. Press Ctrl-C to stop.");

    loop {
        match waitpid(None, Some(WaitPidFlag::__WALL)) {
            Ok(status) => handle_wait_status(&mut traced, status),
            Err(nix::errno::Errno::ECHILD) => {
                info!("No more traced processes. Exiting.");
                break;
            }
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => {
                error!("waitpid failed: {}", e);
                break;
            }
        }
    }

    Ok(())
}

fn handle_wait_status(traced: &mut HashSet<Pid>, status: WaitStatus) {
    match status {
        WaitStatus::PtraceEvent(pid, _sig, event) => {
            handle_ptrace_event(traced, pid, event);
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
            traced.remove(&pid);
        }
        WaitStatus::Signaled(pid, sig, _core) => {
            info!("[exit] PID {} killed by {:?}", pid, sig);
            traced.remove(&pid);
        }
        other => {
            debug!("PID {:?}: {:?}", other.pid(), other);
            if let Some(pid) = other.pid() {
                let _ = ptrace::cont(pid, None);
            }
        }
    }
}

fn handle_ptrace_event(traced: &mut HashSet<Pid>, pid: Pid, event: i32) {
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
                    traced.insert(child_pid);
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
            let cmdline = read_cmdline(pid)
                .map(|args| shell_join(&args))
                .unwrap_or_else(|| "<unavailable>".into());
            info!("[exec] PID {}: {}", pid, cmdline);
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
