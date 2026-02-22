use std::fs;

use anyhow::{bail, Context, Result};
use log::{info, warn};
use nix::sys::ptrace;
use nix::unistd::Pid;

use crate::nixutil;

/// The ptrace options we set on every tracee.
fn trace_options() -> ptrace::Options {
    ptrace::Options::PTRACE_O_TRACEFORK
        | ptrace::Options::PTRACE_O_TRACEVFORK
        | ptrace::Options::PTRACE_O_TRACECLONE
        | ptrace::Options::PTRACE_O_TRACEEXEC
}

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
        if let Some(args) = nixutil::read_cmdline(pid) {
            if args.len() >= 2 && args[0] == "nix-daemon" && args[1] == "--daemon" {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

/// Find all nix-daemon processes and attach to them with ptrace.
/// Returns the number of successfully attached processes.
pub fn attach_to_nix_daemons() -> Result<usize> {
    let daemon_pids = find_nix_daemon_pids()?;
    if daemon_pids.is_empty() {
        bail!("No nix-daemon processes found (looking for cmdline 'nix-daemon --daemon')");
    }

    let mut attached = 0usize;

    for &pid in &daemon_pids {
        match ptrace::seize(pid, trace_options()) {
            Ok(()) => {
                info!("Attached to nix-daemon (pid {})", pid);
                attached += 1;
            }
            Err(e) => {
                warn!("Failed to attach to pid {}: {} (are you root?)", pid, e);
            }
        }
    }

    if attached == 0 {
        bail!("Failed to attach to any nix-daemon process");
    }

    Ok(attached)
}
