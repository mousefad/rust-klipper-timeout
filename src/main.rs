mod cli;
mod config;
mod daemon;

use anyhow::{Context, Result};
use cli::{Cli, init_tracing};
use config::{Config, load_config};
use daemon::{ClipboardDaemon, KlipperProxy};
use zbus::Connection;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse_args();
    init_tracing(cli.verbosity()).context("initializing logging")?;

    let file_cfg = load_config()?;
    let config = Config::from_sources(file_cfg, &cli)?;

    let connection = Connection::session()
        .await
        .context("connecting to D-Bus session bus")?;
    let proxy = KlipperProxy::new(&connection)
        .await
        .context("creating Klipper D-Bus proxy")?;

    let mut daemon = ClipboardDaemon::new(config, proxy).await?;
    daemon.run().await
}
