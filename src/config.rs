use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use dirs::config_dir;
use regex::Regex;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::cli::Cli;

#[derive(Debug, Default, Deserialize)]
pub struct FileConfig {
    pub item_expiry_seconds: Option<u64>,
    pub update_interval_seconds: Option<u64>,
    #[serde(default)]
    pub never_expire_regex: Vec<String>,
    #[serde(default)]
    pub exclude_regex: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub expiry: Duration,
    pub resync: Duration,
    pub exclude: Vec<Regex>,
    pub always_remove: Vec<Regex>,
}

impl Config {
    const DEFAULT_EXPIRY: Duration = Duration::from_secs(10 * 60);
    const DEFAULT_RESYNC: Duration = Duration::from_secs(30);

    pub fn from_sources(file: Option<FileConfig>, cli: &Cli) -> Result<Self> {
        debug!(?cli, ?file, "resolved configuration inputs");

        let file = file.unwrap_or_default();

        let always_remove = Self::compile_patterns(
            &cli.exclude_regex
                .iter()
                .chain(file.exclude_regex.iter())
                .collect()
        ).context("parsing exclude_regex pattern")?;

        let never_remove = Self::compile_patterns(
            &cli.never_expire_regex
                .iter()
                .chain(file.never_expire_regex.iter())
                .collect()
        ).context("parsing never_expire_regex")?;

        let expiry_secs = cli
            .item_expiry_seconds
            .or(file.item_expiry_seconds)
            .unwrap_or(Self::DEFAULT_EXPIRY.as_secs());
        if expiry_secs == 0 {
            bail!("expiry_seconds must be greater than zero");
        }

        let resync_secs = cli
            .update_interval_seconds
            .or(file.update_interval_seconds)
            .unwrap_or(Self::DEFAULT_RESYNC.as_secs());
        if resync_secs == 0 {
            bail!("resync_interval_seconds must be greater than zero");
        }

        let config = Self {
            expiry: Duration::from_secs(expiry_secs),
            resync: Duration::from_secs(resync_secs),
            always_remove,
            exclude: never_remove,
        };
        debug!(?config, "using merged configuration");
        Ok(config)
    }

    fn compile_patterns(patterns: &Vec<&String>) -> Result<Vec<Regex>> {
        patterns
            .iter()
            .map(|pattern| Regex::new(pattern).with_context(|| format!("invalid regex: {pattern}")))
            .collect()
    }

    pub fn should_always_remove(&self, content: &str) -> bool {
        self.always_remove
            .iter()
            .any(|regex| regex.is_match(content))
    }

    pub fn should_never_remove(&self, content: &str) -> bool {
        self.exclude
            .iter()
            .any(|regex| regex.is_match(content))
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
