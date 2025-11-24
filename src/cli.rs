use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

/// Command-line interface definition.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Cli {
    /// Seconds before a clipboard entry is purged.
    #[arg(long)]
    pub expiry_seconds: Option<u64>,

    /// How often to resync the clipboard history from Klipper (seconds).
    #[arg(long)]
    pub resync_interval_seconds: Option<u64>,

    /// Log more verbosely. Use up to three times for increasingly verbose output.
    #[arg(long, short, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }

    pub fn verbosity(&self) -> u8 {
        self.verbose
    }
}

pub fn init_tracing(verbose: u8) -> Result<()> {
    let log_level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        3 => "trace",
        _ => {
            info!("already at maximum --verbose level");
            "trace"
        }
    };

    let env_filter = EnvFilter::try_new(log_level)?;
    fmt().with_env_filter(env_filter).init();
    Ok(())
}
