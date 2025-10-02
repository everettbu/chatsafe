use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use chatsafe_common::{GenerationParams, Result, Error};

/// Complete model configuration from registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Model file path/name
    pub path: String,
    /// Context window size
    pub ctx_window: usize,
    /// Template identifier
    pub template_id: String,
    /// Stop sequences
    pub stop_sequences: Vec<String>,
    /// End of sequence token
    pub eos_token: String,
    /// Default generation parameters
    pub defaults: ModelDefaults,
    /// Resource requirements
    pub resources: ModelResources,
    /// Whether this is the default model
    pub default: bool,
    /// Model-specific metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Default generation parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDefaults {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub repeat_penalty: f32,
    pub max_tokens: usize,
}

/// Resource requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResources {
    /// Minimum RAM in GB
    pub min_ram_gb: f32,
    /// Estimated disk space in GB
    pub est_disk_gb: f32,
    /// GPU layers (-1 for all, 0 for CPU only)
    pub gpu_layers: i32,
    /// Recommended thread count
    pub threads: usize,
}

/// Template configuration for different model families
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    pub id: String,
    pub name: String,
    pub system_prefix: String,
    pub system_suffix: String,
    pub user_prefix: String,
    pub user_suffix: String,
    pub assistant_prefix: String,
    pub assistant_suffix: String,
    pub default_system_prompt: String,
}

/// Registry containing models and templates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRegistryData {
    pub version: String,
    pub templates: Vec<TemplateConfig>,
    pub models: Vec<ModelConfig>,
}

/// Model registry manager
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: HashMap<String, ModelConfig>,
    templates: HashMap<String, TemplateConfig>,
    model_dir: PathBuf,
    default_model_id: Option<String>,
}

impl ModelRegistry {
    /// Create an empty registry
    pub fn new() -> Result<Self> {
        let model_dir = dirs::home_dir()
            .ok_or_else(|| Error::ConfigError("Cannot determine home directory".into()))?
            .join(".local/share/chatsafe/models");
        
        Ok(Self {
            models: HashMap::new(),
            templates: HashMap::new(),
            model_dir,
            default_model_id: None,
        })
    }
    
    /// Load registry from JSON file
    pub fn load_from_file(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let data: ModelRegistryData = serde_json::from_str(&content)?;
        Self::from_data(data)
    }
    
    /// Load registry from JSON string
    pub fn load_from_json(json: &str) -> Result<Self> {
        let data: ModelRegistryData = serde_json::from_str(json)?;
        Self::from_data(data)
    }
    
    /// Create registry from data
    fn from_data(data: ModelRegistryData) -> Result<Self> {
        let mut registry = Self::new()?;
        
        // Load templates
        for template in data.templates {
            registry.templates.insert(template.id.clone(), template);
        }
        
        // Load models and find default
        let mut default_found = false;
        for model in data.models {
            if model.default {
                if default_found {
                    return Err(Error::ConfigError("Multiple default models specified".into()));
                }
                registry.default_model_id = Some(model.id.clone());
                default_found = true;
            }
            registry.models.insert(model.id.clone(), model);
        }
        
        if !default_found && !registry.models.is_empty() {
            // If no default specified, use first model
            let first_id = registry.models.keys().next().cloned();
            if let Some(id) = first_id {
                registry.default_model_id = Some(id.clone());
                if let Some(model) = registry.models.get_mut(&id) {
                    model.default = true;
                }
            }
        }
        
        Ok(registry)
    }
    
    /// Load default registry with built-in models
    pub fn load_defaults() -> Result<Self> {
        let json = include_str!("default_registry.json");
        Self::load_from_json(json)
    }
    
    /// Get a model by ID
    pub fn get_model(&self, id: &str) -> Result<&ModelConfig> {
        self.models
            .get(id)
            .ok_or_else(|| Error::ModelNotFound(id.to_string()))
    }
    
    /// Get the default model
    pub fn get_default_model(&self) -> Result<&ModelConfig> {
        let id = self.default_model_id
            .as_ref()
            .ok_or_else(|| Error::ConfigError("No default model configured".into()))?;
        self.get_model(id)
    }
    
    /// Get a template by ID
    pub fn get_template(&self, id: &str) -> Result<&TemplateConfig> {
        self.templates
            .get(id)
            .ok_or_else(|| Error::ConfigError(format!("Template not found: {}", id)))
    }
    
    /// Get template for a model
    pub fn get_model_template(&self, model_id: &str) -> Result<&TemplateConfig> {
        let model = self.get_model(model_id)?;
        self.get_template(&model.template_id)
    }
    
    /// Get the full path to a model file
    pub fn get_model_path(&self, model_id: &str) -> Result<PathBuf> {
        let model = self.get_model(model_id)?;
        Ok(self.model_dir.join(&model.path))
    }
    
    /// Set model directory
    pub fn set_model_dir(&mut self, dir: PathBuf) {
        self.model_dir = dir;
    }
    
    /// Create generation params from model defaults
    pub fn get_generation_params(&self, model_id: &str) -> Result<GenerationParams> {
        let model = self.get_model(model_id)?;
        Ok(GenerationParams {
            request_id: uuid::Uuid::new_v4().to_string(),
            temperature: model.defaults.temperature,
            max_tokens: model.defaults.max_tokens,
            top_p: model.defaults.top_p,
            top_k: model.defaults.top_k,
            repeat_penalty: model.defaults.repeat_penalty,
            stop_sequences: model.stop_sequences.clone(),
        })
    }
    
    /// Apply request overrides to generation params
    pub fn apply_overrides(
        &self,
        model_id: &str,
        temperature: Option<f32>,
        max_tokens: Option<usize>,
        top_p: Option<f32>,
        top_k: Option<i32>,
        repeat_penalty: Option<f32>,
    ) -> Result<GenerationParams> {
        let mut params = self.get_generation_params(model_id)?;
        
        if let Some(t) = temperature {
            params.temperature = t;
        }
        if let Some(m) = max_tokens {
            params.max_tokens = m;
        }
        if let Some(p) = top_p {
            params.top_p = p;
        }
        if let Some(k) = top_k {
            params.top_k = k;
        }
        if let Some(r) = repeat_penalty {
            params.repeat_penalty = r;
        }
        
        Ok(params)
    }
    
    /// List all available model IDs
    pub fn list_models(&self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }
    
    /// Check if system has sufficient resources for a model
    pub fn check_resources(&self, model_id: &str) -> Result<bool> {
        let model = self.get_model(model_id)?;
        let resources = &model.resources;
        
        // Simple check - could be enhanced with actual system resource detection
        let sys_info = sys_info::mem_info()
            .map_err(|e| Error::Internal(format!("Failed to get system info: {}", e)))?;
        
        let available_ram_gb = (sys_info.avail as f32) / (1024.0 * 1024.0 * 1024.0);
        
        Ok(available_ram_gb >= resources.min_ram_gb)
    }
    
    /// Export registry to JSON
    pub fn export(&self) -> Result<String> {
        let data = ModelRegistryData {
            version: "1.0".to_string(),
            templates: self.templates.values().cloned().collect(),
            models: self.models.values().cloned().collect(),
        };
        
        serde_json::to_string_pretty(&data)
            .map_err(Error::Serialization)
    }
}

// For the sys_info functionality
mod sys_info {
    use chatsafe_common::{Result, Error};
    
    pub struct MemInfo {
        pub _total: u64,
        pub avail: u64,
    }
    
    pub fn mem_info() -> Result<MemInfo> {
        // Platform-specific implementation
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            
            let output = Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .map_err(|e| Error::Internal(format!("Failed to get memory info: {}", e)))?;
            
            let total_str = String::from_utf8_lossy(&output.stdout);
            let total = total_str.trim().parse::<u64>()
                .map_err(|e| Error::Internal(format!("Failed to parse memory: {}", e)))?;
            
            // For available memory, use vm_stat
            let output = Command::new("vm_stat")
                .output()
                .map_err(|e| Error::Internal(format!("Failed to get vm_stat: {}", e)))?;
            
            let vm_stat = String::from_utf8_lossy(&output.stdout);
            let mut free_pages = 0u64;
            let mut page_size = 4096u64; // default
            
            for line in vm_stat.lines() {
                if line.contains("page size of") {
                    if let Some(size_str) = line.split_whitespace().last() {
                        page_size = size_str.trim().parse().unwrap_or(4096);
                    }
                } else if line.starts_with("Pages free:") {
                    if let Some(pages_str) = line.split(':').nth(1) {
                        free_pages = pages_str.trim().trim_end_matches('.').parse().unwrap_or(0);
                    }
                }
            }
            
            Ok(MemInfo {
                _total: total,
                avail: free_pages * page_size,
            })
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            // Simple fallback for other platforms
            Ok(MemInfo {
                _total: 8 * 1024 * 1024 * 1024, // 8GB default
                avail: 4 * 1024 * 1024 * 1024, // 4GB available
            })
        }
    }
}