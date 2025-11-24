use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::time;
use tracing::{debug, error, info, warn};
use zbus::{Connection, Proxy, proxy::SignalStream};

use crate::config::Config;

#[derive(Debug, Clone)]
struct TrackedEntry {
    content: String,
    first_seen: Instant,
}

pub struct KlipperProxy<'conn> {
    inner: Proxy<'conn>,
}

impl<'conn> KlipperProxy<'conn> {
    pub async fn new(connection: &'conn Connection) -> zbus::Result<Self> {
        let proxy = Proxy::new(
            connection,
            "org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper",
        )
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

pub struct ClipboardDaemon<'conn> {
    config: Config,
    proxy: KlipperProxy<'conn>,
    entries: Vec<TrackedEntry>,
}

impl<'conn> ClipboardDaemon<'conn> {
    pub async fn new(config: Config, proxy: KlipperProxy<'conn>) -> Result<Self> {
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
