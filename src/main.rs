mod daemon;
mod limiter;
mod nixutil;
mod tracer;

use anyhow::Result;
use clap::Parser;
use log::{error, info};
use nix::sys::wait::{waitpid, WaitPidFlag};

use tracer::Tracer;

/// Trace all programs execve'd by the Nix daemon and limit concurrency.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Maximum number of concurrent rate-limited processes [default: number of CPU cores].
    #[arg(short = 'j', long, default_value_t = default_concurrency())]
    max_concurrent: usize,
}

fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    daemon::attach_to_nix_daemons()?;

    let mut tracer = Tracer::new(args.max_concurrent);

    info!("Tracing started (max concurrency: {}). Press Ctrl-C to stop.", args.max_concurrent);

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
