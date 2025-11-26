use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::time;
use tracing::{debug, error, info, warn};
use zbus::{Connection, proxy};

use crate::config::Config;

#[proxy(
    interface = "org.kde.klipper.klipper",
    default_service = "org.kde.klipper",
    default_path = "/klipper"
)]
pub trait Klipper {
    #[zbus(name = "getClipboardHistoryMenu")]
    async fn get_clipboard_history(&self) -> zbus::Result<Vec<String>>;

    #[zbus(name = "clearClipboardHistory")]
    async fn clear_clipboard_history(&self) -> zbus::Result<()>;

    #[zbus(name = "setClipboardContents")]
    async fn set_clipboard_contents(&self, contents: &str) -> zbus::Result<()>;

    #[zbus(signal, name = "clipboardHistoryUpdated")]
    fn clipboard_history_updated(&self) -> zbus::Result<()>;
}

#[derive(Debug, Clone)]
struct TrackedEntry {
    content: String,
    first_seen: Instant,
}

pub struct ClipboardDaemon<'conn> {
    config: Config,
    proxy: KlipperProxy<'conn>,
    entries: Vec<TrackedEntry>,
}

impl<'conn> ClipboardDaemon<'conn> {
    pub async fn new(config: Config, connection: &'conn Connection) -> Result<Self> {
        let proxy = KlipperProxy::new(connection)
            .await
            .context("creating Klipper D-Bus proxy")?;
        let mut daemon = Self {
            config,
            proxy,
            entries: Vec::new(),
        };
        daemon.sync_history().await?;
        Ok(daemon)
    }

    pub async fn run(&mut self) -> Result<()> {
        info!(
            expiry = ?self.config.expiry,
            resync = ?self.config.resync,
            "starting clipboard expiry daemon"
        );

        let mut history_stream: Option<clipboardHistoryUpdatedStream> = match self
            .proxy
            .receive_clipboard_history_updated()
            .await
        {
            Ok(stream) => Some(stream),
            Err(err) => {
                warn!(
                    "failed to subscribe to Klipper signals: {err:?}; falling back to polling only"
                );
                None
            }
        };

        let mut resync_tick = time::interval(self.config.resync);
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

        let changed = self.reconcile(history);
        if changed {
            self.rewrite_history()
                .await
                .context("rewriting clipboard history after filtering")?;
        }
        Ok(())
    }

    fn reconcile(&mut self, history: Vec<String>) -> bool {
        let mut matched = vec![false; self.entries.len()];
        let mut next = Vec::with_capacity(history.len());
        let now = Instant::now();
        let mut filtered = false;

        for content in history {
            if self.config.should_always_remove(&content) {
                info!("removing clipboard entry that matches always_remove_patterns");
                filtered = true;
                continue;
            }
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
        filtered
    }

    async fn expire_due_entries(&mut self) -> Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }

        let expiry = self.config.expiry;
        let mut changed = false;

        self.entries.retain(|entry| {
            let expired = entry.first_seen.elapsed() >= expiry;
            if expired && self.config.should_never_remove(&entry.content) {
                debug!("skipping expiry for clipboard entry that matches never_remove_patterns");
                return true;
            }
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
