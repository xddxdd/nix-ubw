mod limiter;
mod nixutil;
mod tracer;

use std::fs;

use anyhow::{bail, Context, Result};
use clap::Parser;
use log::{error, info, warn};
use nix::sys::ptrace;
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::Pid;

use tracer::{read_cmdline, Tracer};

/// Trace all programs execve'd by the Nix daemon and limit concurrency.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Maximum number of concurrent rate-limited processes.
    #[arg(short = 'j', long, default_value_t = 10)]
    max_concurrent: usize,
}

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

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    let daemon_pids = find_nix_daemon_pids()?;
    if daemon_pids.is_empty() {
        bail!("No nix-daemon processes found (looking for cmdline 'nix-daemon --daemon')");
    }

    let mut tracer = Tracer::new(args.max_concurrent);

    for &pid in &daemon_pids {
        match ptrace::seize(pid, trace_options()) {
            Ok(()) => {
                info!("Attached to nix-daemon (pid {})", pid);
                tracer.traced.insert(pid);
            }
            Err(e) => {
                warn!("Failed to attach to pid {}: {} (are you root?)", pid, e);
            }
        }
    }

    if tracer.traced.is_empty() {
        bail!("Failed to attach to any nix-daemon process");
    }

    info!("Tracing started. Press Ctrl-C to stop.");

    loop {
        match waitpid(None, Some(WaitPidFlag::__WALL)) {
            Ok(status) => tracer.handle_wait_status(status),
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
