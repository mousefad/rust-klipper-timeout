use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use dirs::config_dir;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::time;
use tracing::{debug, error, info, warn};
use zbus::{Connection, Proxy, ProxyBuilder, proxy::SignalStream};

#[derive(Parser, Debug)]
#[command(author, version, about = "Time-based clipboard expiry for Klipper", long_about = None)]
struct Cli {
    /// Optional path to a TOML config file (defaults to ~/.config/klipper-timeout/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Seconds before a clipboard entry is purged
    #[arg(long)]
    expiry_seconds: Option<u64>,

    /// How often to resync the clipboard history from Klipper (seconds)
    #[arg(long)]
    resync_interval_seconds: Option<u64>,

    /// Tracing/ logging directives (e.g. info,debug,trace)
    #[arg(long)]
    log_level: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    expiry_seconds: Option<u64>,
    resync_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
struct Config {
    expiry: Duration,
    resync: Duration,
}

impl Config {
    const DEFAULT_EXPIRY: Duration = Duration::from_secs(10 * 60);
    const DEFAULT_RESYNC: Duration = Duration::from_secs(30);

    fn from_sources(file: Option<FileConfig>, cli: &Cli) -> Result<Self> {
        debug!("CLI Options: {:?}, cli", cli);
        debug!("File Config: {:?}, file", file);
        let file = file.unwrap_or_default();

        let expiry_secs = cli
            .expiry_seconds
            .or(file.expiry_seconds)
            .unwrap_or(Self::DEFAULT_EXPIRY.as_secs());
        if expiry_secs == 0 {
            bail!("expiry_seconds must be greater than zero");
        }

        let resync_secs = cli
            .resync_interval_seconds
            .or(file.resync_interval_seconds)
            .unwrap_or(Self::DEFAULT_RESYNC.as_secs());
        if resync_secs == 0 {
            bail!("resync_interval_seconds must be greater than zero");
        }

        let config = Self{
            expiry: Duration::from_secs(expiry_secs),
            resync: Duration::from_secs(resync_secs),
        };
        debug!("Use Config:  {:?}", config);
        Ok(config)
    }
}

#[derive(Debug, Clone)]
struct TrackedEntry {
    content: String,
    first_seen: Instant,
}

struct KlipperProxy<'conn> {
    inner: Proxy<'conn>,
}

impl<'conn> KlipperProxy<'conn> {
    async fn new(connection: &'conn Connection) -> zbus::Result<Self> {
        let proxy = ProxyBuilder::new(connection)
            .destination("org.kde.klipper")?
            .path("/klipper")?
            .interface("org.kde.klipper.klipper")?
            .build()
            .await?;
        Ok(Self { inner: proxy })
    }

    async fn get_clipboard_history(&self) -> zbus::Result<Vec<String>> {
        self.inner.call("getClipboardHistoryMenu", &()).await
    }

    async fn clear_clipboard_history(&self) -> zbus::Result<()> {
        self.inner.call("clearClipboardHistory", &()).await
    }

    async fn set_clipboard_contents(&self, contents: &str) -> zbus::Result<()> {
        self.inner.call("setClipboardContents", &(contents,)).await
    }

    async fn receive_clipboard_history_updated(&self) -> zbus::Result<SignalStream<'conn>> {
        self.inner.receive_signal("clipboardHistoryUpdated").await
    }
}

struct ClipboardDaemon<'conn> {
    config: Config,
    proxy: KlipperProxy<'conn>,
    entries: Vec<TrackedEntry>,
}

impl<'conn> ClipboardDaemon<'conn> {
    async fn new(config: Config, proxy: KlipperProxy<'conn>) -> Result<Self> {
        let mut daemon = Self {
            config,
            proxy,
            entries: Vec::new(),
        };
        daemon.sync_history().await?;
        Ok(daemon)
    }

    async fn run(&mut self) -> Result<()> {
        info!(
            expiry = ?self.config.expiry,
            resync = ?self.config.resync,
            "starting clipboard expiry daemon"
        );

        let mut history_stream = match self.proxy.receive_clipboard_history_updated().await {
            Ok(stream) => Some(stream),
            Err(err) => {
                warn!(
                    "failed to subscribe to Klipper signals: {err:?}; falling back to polling only"
                );
                None
            }
        };

        let mut resync_tick = time::interval(self.config.resync);
        // start ticking immediately
        resync_tick.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        let mut expiry_tick = time::interval(Duration::from_secs(1));
        expiry_tick.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                res = shutdown.as_mut() => {
                    if let Err(err) = res {
                        error!("failed to listen for shutdown signal: {err:?}");
                    }
                    info!("shutting down per ctrl-c");
                    break;
                }
                _ = resync_tick.tick() => {
                    if let Err(err) = self.sync_history().await {
                        warn!("refreshing clipboard history failed: {err:?}");
                    }
                }
                _ = expiry_tick.tick() => {
                    if let Err(err) = self.expire_due_entries().await {
                        warn!("failed to expire entries: {err:?}");
                    }
                }
                Some(_) = async {
                    if let Some(ref mut stream) = history_stream {
                        stream.next().await
                    } else {
                        None
                    }
                } => {
                    if let Err(err) = self.sync_history().await {
                        warn!("failed to process clipboard update: {err:?}");
                    }
                }
            }
        }

        Ok(())
    }

    async fn sync_history(&mut self) -> Result<()> {
        let history = self
            .proxy
            .get_clipboard_history()
            .await
            .context("fetching clipboard history from Klipper")?;

        self.reconcile(history);
        Ok(())
    }

    fn reconcile(&mut self, history: Vec<String>) {
        let mut matched = vec![false; self.entries.len()];
        let mut next = Vec::with_capacity(history.len());
        let now = Instant::now();

        for content in history {
            if let Some((idx, entry)) = self
                .entries
                .iter()
                .enumerate()
                .find(|(i, entry)| !matched[*i] && entry.content == content)
            {
                matched[idx] = true;
                next.push(entry.clone());
            } else {
                debug!("tracking new clipboard entry");
                next.push(TrackedEntry {
                    content,
                    first_seen: now,
                });
            }
        }

        self.entries = next;
    }

    async fn expire_due_entries(&mut self) -> Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        // Why create a copy insead of just using from struct?
        let expiry = self.config.expiry;
        let mut changed = false;

        self.entries.retain(|entry| {
            let expired = entry.first_seen.elapsed() >= expiry;
            if expired {
                info!(
                    age = ?entry.first_seen.elapsed(),
                    "expiring clipboard entry",
                );
                changed = true;
            }
            !expired
        });

        if changed {
            self.rewrite_history()
                .await
                .context("rewriting clipboard history")?;
        }

        Ok(())
    }

    async fn rewrite_history(&self) -> Result<()> {
        self.proxy
            .clear_clipboard_history()
            .await
            .context("clearing clipboard history")?;

        for entry in self.entries.iter().rev() {
            self.proxy
                .set_clipboard_contents(&entry.content)
                .await
                .with_context(|| "restoring clipboard entry")?;
        }

        Ok(())
    }
}

fn default_config_path(cli_path: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(path) = cli_path {
        return Some(path.clone());
    }
    config_dir().map(|dir| dir.join("klipper-timeout").join("config.toml"))
}

fn load_config(cli_path: Option<&PathBuf>) -> Result<Option<FileConfig>> {
    let path = match default_config_path(cli_path) {
        Some(path) => path,
        None => return Ok(None),
    };

    if !path.exists() {
        debug!("Config file does not exist: {:?}", path);
        return Ok(None);
    } else {
        debug!("Reading from config file: {:?}", path);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("reading config file at {}", path.display()))?;
    let parsed: FileConfig = toml::from_str(&content)
        .with_context(|| format!("parsing config file at {}", path.display()))?;
    Ok(Some(parsed))
}

fn init_tracing(log_level: Option<&str>) -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt};

    let env_filter = if let Some(level) = log_level {
        EnvFilter::try_new(level)?
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    fmt().with_env_filter(env_filter).init();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log_level.as_deref()).context("initializing logging")?;

    let file_cfg = load_config(cli.config.as_ref())?;
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
