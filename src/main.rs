mod daemon;
mod limiter;
mod nixutil;
mod resources;
mod tracer;

use std::fs;

use anyhow::{Context, Result};
use clap::Parser;
use log::{error, info};
use nix::sys::wait::{waitpid, WaitPidFlag};

use resources::ResourceProfile;
use tracer::Tracer;

/// Trace all programs execve'd by the Nix daemon and throttle resource-intensive ones.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Total CPU cores available for throttled processes [default: system core count].
    #[arg(short = 'c', long, default_value_t = default_cpus())]
    total_cpus: i32,

    /// Total memory in GiB available for throttled processes [default: system RAM, rounded down].
    #[arg(short = 'm', long, default_value_t = default_mem_gb())]
    total_mem_gb: i32,
}

fn default_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
}

/// Read total system RAM from /proc/meminfo, returned in GiB (rounded down).
fn default_mem_gb() -> i32 {
    (|| -> Option<i32> {
        let data = fs::read_to_string("/proc/meminfo").ok()?;
        for line in data.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                // Format: "MemTotal:    16348160 kB"
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some((kb / (1024 * 1024)) as i32);
            }
        }
        None
    })()
    .unwrap_or(8)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    let total_budget = ResourceProfile::new(args.total_cpus, args.total_mem_gb);

    daemon::attach_to_nix_daemons().context("Failed to attach to nix-daemon")?;

    info!(
        "Tracing started — budget: {}. Press Ctrl-C to stop.",
        total_budget
    );

    let mut tracer = Tracer::new(total_budget);

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
