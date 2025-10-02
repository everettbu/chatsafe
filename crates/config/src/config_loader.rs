use chatsafe_common::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub runtime: RuntimeConfig,
    pub models: ModelsConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
}

/// Runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub llama_server_port: u16,
    pub threads: usize,
    pub gpu_layers: Option<i32>,
}

/// Models configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    pub directory: PathBuf,
    pub registry_file: Option<PathBuf>,
    pub default_model: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8081,
                max_connections: 100,
            },
            runtime: RuntimeConfig {
                llama_server_port: 8080,
                threads: 4,
                gpu_layers: None, // Auto-detect
            },
            models: ModelsConfig {
                directory: dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/share/chatsafe/models"),
                registry_file: None,
                default_model: "llama-3.2-3b-instruct-q4_k_m".to_string(),
            },
        }
    }
}

/// Configuration loader
pub struct ConfigLoader;

impl ConfigLoader {
    /// Load configuration from file or use defaults
    pub fn load(path: Option<&PathBuf>) -> Result<AppConfig> {
        if let Some(path) = path {
            if path.exists() {
                let content = std::fs::read_to_string(path)?;
                let config: AppConfig = serde_json::from_str(&content)?;
                return Ok(config);
            }
        }

        // Check default locations
        let default_paths = vec![
            PathBuf::from("chatsafe.json"),
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("chatsafe/config.json"),
        ];

        for path in default_paths {
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let config: AppConfig = serde_json::from_str(&content)?;
                return Ok(config);
            }
        }

        // Use defaults
        Ok(AppConfig::default())
    }

    /// Save configuration to file
    pub fn save(config: &AppConfig, path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string_pretty(config)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
