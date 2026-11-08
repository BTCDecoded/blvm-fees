//! `<data dir>/config.toml`. The priority array stays in this file.

use serde::{Deserialize, Serialize};

pub const DEFAULT_MAX_HEIGHT_DELTA: u64 = 3;
pub const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 5_000;
pub const DEFAULT_SUBMIT_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeesConfig {
    pub priority: Vec<String>,
    pub max_height_delta: u64,
    pub request_timeout_ms: u64,
    pub submit_interval_secs: u64,
}

impl Default for FeesConfig {
    fn default() -> Self {
        Self {
            priority: vec![
                "mempool-space".to_string(),
                "native".to_string(),
                "blockstream".to_string(),
            ],
            max_height_delta: DEFAULT_MAX_HEIGHT_DELTA,
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
            submit_interval_secs: DEFAULT_SUBMIT_INTERVAL_SECS,
        }
    }
}

impl FeesConfig {
    pub fn load(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn apply_scalar_overrides(
        &mut self,
        max_height_delta: Option<&str>,
        request_timeout_ms: Option<&str>,
        submit_interval_secs: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(text) = max_height_delta {
            self.max_height_delta = text
                .parse()
                .map_err(|_| anyhow::anyhow!("max_height_delta must be an integer, got {text}"))?;
        }
        if let Some(text) = request_timeout_ms {
            self.request_timeout_ms = text.parse().map_err(|_| {
                anyhow::anyhow!("request_timeout_ms must be an integer, got {text}")
            })?;
        }
        if let Some(text) = submit_interval_secs {
            self.submit_interval_secs = text.parse().map_err(|_| {
                anyhow::anyhow!("submit_interval_secs must be an integer, got {text}")
            })?;
        }
        self.validate()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.submit_interval_secs < 1 {
            anyhow::bail!(
                "submit_interval_secs must be at least 1, got {}",
                self.submit_interval_secs
            );
        }
        if self.request_timeout_ms == 0 {
            anyhow::bail!("request_timeout_ms must be at least 1, got 0");
        }
        Ok(())
    }
}
