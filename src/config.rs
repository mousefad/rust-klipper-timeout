use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use dirs::config_dir;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::cli::Cli;

#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    pub expiry_seconds: Option<u64>,
    pub resync_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub expiry: Duration,
    pub resync: Duration,
}

impl Config {
    const DEFAULT_EXPIRY: Duration = Duration::from_secs(10 * 60);
    const DEFAULT_RESYNC: Duration = Duration::from_secs(30);

    pub fn from_sources(file: Option<FileConfig>, cli: &Cli) -> Result<Self> {
        debug!(?cli, ?file, "resolved configuration inputs");

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

        let config = Self {
            expiry: Duration::from_secs(expiry_secs),
            resync: Duration::from_secs(resync_secs),
        };
        debug!(?config, "using merged configuration");
        Ok(config)
    }
}

pub fn load_config() -> Result<Option<FileConfig>> {
    let path = match default_config_path() {
        Some(path) => path,
        None => {
            warn!("could not determine path config file");
            return Ok(None);
        }
    };

    if !path.exists() {
        debug!("config file does not exist: {:?}", path);
        return Ok(None);
    } else {
        debug!("reading from config file: {:?}", path);
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("reading config file at {}", path.display()))?;

    let parsed = toml::from_str(&content)
        .with_context(|| format!("parsing config file at {}", path.display()))?;
    Ok(Some(parsed))
}

fn default_config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("klipper-timeout.toml"))
}
