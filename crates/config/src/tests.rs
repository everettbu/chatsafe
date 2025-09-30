#[cfg(test)]
mod tests {
    use crate::model_registry::*;
    use chatsafe_common::Result;
    
    #[test]
    fn test_load_default_registry() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        // Check we have models
        assert!(!registry.list_models().is_empty());
        
        // Check default model exists
        let default_model = registry.get_default_model()?;
        assert_eq!(default_model.id, "llama-3.2-3b-instruct-q4_k_m");
        assert!(default_model.default);
        
        Ok(())
    }
    
    #[test]
    fn test_model_configuration() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        let model = registry.get_model("llama-3.2-3b-instruct-q4_k_m")?;
        assert_eq!(model.name, "Llama 3.2 3B Instruct (Q4_K_M)");
        assert_eq!(model.ctx_window, 8192);
        assert_eq!(model.template_id, "llama3");
        assert_eq!(model.eos_token, "<|eot_id|>");
        
        // Check defaults
        assert_eq!(model.defaults.temperature, 0.6);
        assert_eq!(model.defaults.max_tokens, 256);
        assert_eq!(model.defaults.top_p, 0.9);
        assert_eq!(model.defaults.top_k, 40);
        assert_eq!(model.defaults.repeat_penalty, 1.15);
        
        // Check resources
        assert_eq!(model.resources.min_ram_gb, 3.0);
        assert_eq!(model.resources.est_disk_gb, 2.0);
        assert_eq!(model.resources.gpu_layers, -1);
        assert_eq!(model.resources.threads, 4);
        
        Ok(())
    }
    
    #[test]
    fn test_template_retrieval() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        // Get llama3 template
        let template = registry.get_template("llama3")?;
        assert_eq!(template.name, "Llama 3 Instruct");
        assert!(template.system_prefix.contains("system"));
        assert!(template.user_prefix.contains("user"));
        assert!(template.assistant_prefix.contains("assistant"));
        
        // Get template for model
        let template = registry.get_model_template("llama-3.2-3b-instruct-q4_k_m")?;
        assert_eq!(template.id, "llama3");
        
        Ok(())
    }
    
    #[test]
    fn test_generation_params() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        // Get default params for model
        let params = registry.get_generation_params("llama-3.2-3b-instruct-q4_k_m")?;
        assert_eq!(params.temperature, 0.6);
        assert_eq!(params.max_tokens, 256);
        assert_eq!(params.stop_sequences.len(), 3);
        assert!(!params.request_id.is_empty());
        
        Ok(())
    }
    
    #[test]
    fn test_apply_overrides() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        // Apply overrides
        let params = registry.apply_overrides(
            "llama-3.2-3b-instruct-q4_k_m",
            Some(0.8),  // temperature
            Some(512),  // max_tokens
            None,       // top_p - use default
            Some(50),   // top_k
            None,       // repeat_penalty - use default
        )?;
        
        assert_eq!(params.temperature, 0.8);
        assert_eq!(params.max_tokens, 512);
        assert_eq!(params.top_p, 0.9); // default
        assert_eq!(params.top_k, 50);
        assert_eq!(params.repeat_penalty, 1.15); // default
        
        Ok(())
    }
    
    #[test]
    fn test_multiple_models() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        let models = registry.list_models();
        assert!(models.len() >= 4); // We have at least 4 models in default registry
        
        // Check each model has required fields
        for model_id in &models {
            let model = registry.get_model(model_id)?;
            assert!(!model.id.is_empty());
            assert!(!model.name.is_empty());
            assert!(!model.path.is_empty());
            assert!(model.ctx_window > 0);
            assert!(!model.template_id.is_empty());
            assert!(!model.stop_sequences.is_empty());
            assert!(!model.eos_token.is_empty());
        }
        
        Ok(())
    }
    
    #[test]
    fn test_model_not_found() {
        let registry = ModelRegistry::load_defaults().unwrap();
        
        let result = registry.get_model("nonexistent-model");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            chatsafe_common::Error::ModelNotFound(_)
        ));
    }
    
    #[test]
    fn test_export_registry() -> Result<()> {
        let registry = ModelRegistry::load_defaults()?;
        
        let json = registry.export()?;
        assert!(json.contains("\"version\": \"1.0\""));
        assert!(json.contains("llama-3.2-3b-instruct-q4_k_m"));
        assert!(json.contains("templates"));
        assert!(json.contains("models"));
        
        // Should be valid JSON that can be re-loaded
        let _reloaded = ModelRegistry::load_from_json(&json)?;
        
        Ok(())
    }
    
    #[test]
    fn test_model_path() -> Result<()> {
        let mut registry = ModelRegistry::load_defaults()?;
        
        // Set custom model dir
        registry.set_model_dir(std::path::PathBuf::from("/custom/models"));
        
        let path = registry.get_model_path("llama-3.2-3b-instruct-q4_k_m")?;
        assert_eq!(
            path.to_str().unwrap(),
            "/custom/models/llama-3.2-3b-instruct-q4_k_m.gguf"
        );
        
        Ok(())
    }
}