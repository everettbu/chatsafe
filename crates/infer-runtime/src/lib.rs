use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct InferenceConfig {
    pub model_path: String,
    pub context_size: usize,
    pub threads: usize,
    pub temperature: f32,
    pub max_tokens: usize,
    pub server_port: u16,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            model_path: String::from("models/tinyllama.gguf"),
            context_size: 2048,
            threads: 4,
            temperature: 0.7,
            max_tokens: 512,
            server_port: 8080,
        }
    }
}

pub struct InferenceRuntime {
    config: InferenceConfig,
    server_process: Option<Child>,
    client: reqwest::Client,
}

impl InferenceRuntime {
    pub fn new(config: InferenceConfig) -> Self {
        Self {
            config,
            server_process: None,
            client: reqwest::Client::new(),
        }
    }

    pub async fn start_server(&mut self) -> Result<()> {
        info!("Starting llama.cpp server with model: {}", self.config.model_path);
        
        let mut cmd = Command::new("./llama.cpp/build/bin/llama-server");
        cmd.arg("-m").arg(&self.config.model_path)
           .arg("--host").arg("127.0.0.1")
           .arg("--port").arg(self.config.server_port.to_string())
           .arg("-c").arg(self.config.context_size.to_string())
           .arg("-t").arg(self.config.threads.to_string())
           .arg("--n-gpu-layers").arg("-1") // Use all GPU layers
           .arg("--log-disable")
           .stdout(Stdio::null())
           .stderr(Stdio::null());

        let child = cmd.spawn()
            .context("Failed to start llama.cpp server")?;

        // Wait for server to be ready
        for _ in 0..30 {
            sleep(Duration::from_millis(500)).await;
            if let Ok(resp) = self.client.get(format!("http://127.0.0.1:{}/health", self.config.server_port))
                .send()
                .await 
            {
                if resp.status().is_success() {
                    self.server_process = Some(child);
                    info!("llama.cpp server started successfully");
                    return Ok(());
                }
            }
        }
        
        Err(anyhow::anyhow!("llama.cpp server failed to start"))
    }

    pub async fn stop_server(&mut self) -> Result<()> {
        if let Some(mut process) = self.server_process.take() {
            process.kill().await?;
            info!("llama.cpp server stopped");
        }
        Ok(())
    }
    
    pub async fn complete(&self, prompt: String, temperature: f32, max_tokens: usize) -> Result<CompletionResponse> {
        let request = CompletionRequest {
            prompt,
            temperature: Some(temperature),
            n_predict: Some(max_tokens as i32),
            stream: Some(false),
            cache_prompt: Some(false),
            stop: Some(vec![
                "<|eot_id|>".to_string(),
                "<|end_of_text|>".to_string(),
                "<|start_header_id|>".to_string(),
            ]),
            repeat_penalty: Some(1.15),  // Llama-3 works better with slightly higher penalty
            top_k: Some(40),
            top_p: Some(0.9),
        };
        
        let response = self.client
            .post(format!("http://127.0.0.1:{}/completion", self.config.server_port))
            .json(&request)
            .send()
            .await?
            .json::<CompletionResponse>()
            .await?;
            
        Ok(response)
    }
}

#[derive(Serialize)]
struct CompletionRequest {
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_prompt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub generation_settings: Option<serde_json::Value>,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub stop: Option<bool>,
    pub stopped_eos: Option<bool>,
    pub stopped_limit: Option<bool>,
    pub stopped_word: Option<bool>,
    pub stopping_word: Option<String>,
    pub timings: Option<serde_json::Value>,
    pub tokens_cached: Option<i32>,
    pub tokens_evaluated: Option<i32>,
    pub tokens_predicted: Option<i32>,
    pub truncated: Option<bool>,
}

impl Drop for InferenceRuntime {
    fn drop(&mut self) {
        if let Some(mut process) = self.server_process.take() {
            // Try to kill the process
            let _ = process.start_kill();
        }
    }
}