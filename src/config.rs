use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VigilConfig {
    pub engine: EngineConfig,
    pub api: ApiConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    pub db_path: PathBuf,
    pub max_concurrent_polls: usize,
    pub tick_interval_ms: u64,
    pub retention_days: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub bind_address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for VigilConfig {
    fn default() -> Self {
        Self {
            engine: Default::default(),
            api: Default::default(),
            logging: Default::default(),
        }
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("/var/lib/vigil/vigil.db"),
            max_concurrent_polls: 128,
            tick_interval_ms: 100,
            retention_days: 30,
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            // Spec §8: bind localhost by default
            bind_address: "127.0.0.1:3030".to_string(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config file: {0}")]
    Toml(#[from] toml::de::Error),
}

impl VigilConfig {
    /// Load configuration from a TOML file.
    ///
    /// If the file does not exist, returns defaults.
    /// Environment variable overrides are applied after loading:
    /// - `VIGIL_DB_PATH`
    /// - `VIGIL_BIND_ADDRESS`
    /// - `VIGIL_LOG_LEVEL`
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let mut config = if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            toml::from_str(&contents)?
        } else {
            Self::default()
        };

        // Environment variable overrides.
        if let Ok(v) = std::env::var("VIGIL_DB_PATH") {
            config.engine.db_path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("VIGIL_BIND_ADDRESS") {
            config.api.bind_address = v;
        }
        if let Ok(v) = std::env::var("VIGIL_LOG_LEVEL") {
            config.logging.level = v;
        }

        Ok(config)
    }
}
