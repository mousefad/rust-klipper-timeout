use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

/// Program to add timeout functionality to the KDE/Plasma clipboard manager,
/// Klipper. Configiration is read from `klipper-timeout.toml`, which can be
/// found in the current user's XDG config directory (typically `~/.config/`).
/// 
#[derive(Parser, Debug)]
#[command(version)]
pub struct Cli {
    /// Seconds before a clipboard entries are purged.
    #[arg(long, short='e')]
    pub item_expiry_seconds: Option<u64>,

    /// How often to check for expired items in the clipboard history (seconds).
    #[arg(long, short='i')]
    pub update_interval_seconds: Option<u64>,

    /// Never expire items matching this regex pattern (may be used more
    /// then once).
    #[arg(long, short)]
    pub never_expire_regex: Vec<String>,

    /// Exclude items from the clipboard history if they match this regular
    /// expression (may be used more then once).
    #[arg(long, short='x')]
    pub exclude_regex: Vec<String>,

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
