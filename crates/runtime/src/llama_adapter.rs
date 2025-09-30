use crate::{Runtime, RuntimeHealth, ModelHandle, template_engine::TemplateEngine};
use async_trait::async_trait;
use chatsafe_common::{Message, GenerationParams, Result, Error, StreamFrame, Role, FinishReason, Usage};
use chatsafe_config::{ModelConfig, TemplateConfig, RuntimeConfig};
use futures::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, oneshot};
use tokio::time::{sleep, Duration};
use tracing::{info, warn, debug};

/// Adapter for llama.cpp server
pub struct LlamaAdapter {
    model_path: PathBuf,
    model_config: ModelConfig,
    template_config: TemplateConfig,
    runtime_config: RuntimeConfig,
    server_process: Option<Child>,
    client: Client,
    server_url: String,
    current_handle: Option<ModelHandle>,
    start_time: SystemTime,
    active_requests: Arc<RwLock<std::collections::HashMap<String, oneshot::Sender<()>>>>,
}

impl LlamaAdapter {
    pub fn new(
        model_path: PathBuf,
        model_config: ModelConfig,
        template_config: TemplateConfig,
        runtime_config: RuntimeConfig,
    ) -> Self {
        let server_url = format!("http://127.0.0.1:{}", runtime_config.llama_server_port);
        
        Self {
            model_path,
            model_config,
            template_config,
            runtime_config,
            server_process: None,
            client: Client::builder()
                .timeout(Duration::from_secs(300))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            server_url,
            current_handle: None,
            start_time: SystemTime::now(),
            active_requests: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
    
    fn build_prompt(&self, messages: &[Message]) -> String {
        TemplateEngine::format_prompt(messages, &self.template_config)
    }
    
    fn clean_response(&self, response: &str) -> String {
        let cleaned = TemplateEngine::clean_response(
            response,
            &self.template_config,
            &self.model_config.stop_sequences,
            &self.model_config.eos_token,
        );
        cleaned.content
    }
}

#[async_trait]
impl Runtime for LlamaAdapter {
    async fn load(&mut self, model_id: &str) -> Result<ModelHandle> {
        if model_id != self.model_config.id {
            return Err(Error::InvalidModel(format!(
                "This adapter is configured for model {}, not {}",
                self.model_config.id, model_id
            )));
        }
        
        info!("Loading model: {} from {}", model_id, self.model_path.display());
        
        // Start llama.cpp server
        let mut cmd = Command::new("./llama.cpp/build/bin/llama-server");
        cmd.arg("--model").arg(&self.model_path)
           .arg("--ctx-size").arg(self.model_config.ctx_window.to_string())
           .arg("--n-gpu-layers").arg(self.model_config.resources.gpu_layers.to_string())
           .arg("--host").arg("127.0.0.1")
           .arg("--port").arg(self.runtime_config.llama_server_port.to_string())
           .arg("--threads").arg(self.model_config.resources.threads.to_string())
           .arg("--n-predict").arg("-1")
           .arg("--parallel").arg("4")
           .arg("--cont-batching")
           .arg("--flash-attn").arg("on")
           .stdout(Stdio::piped())
           .stderr(Stdio::piped());
        
        let child = cmd.spawn()
            .map_err(|e| Error::RuntimeError(format!("Failed to start llama.cpp server: {}", e)))?;
        
        self.server_process = Some(child);
        
        // Wait for server to be ready
        for i in 0..30 {
            sleep(Duration::from_millis(500)).await;
            if let Ok(health) = self.health().await {
                if health.is_healthy {
                    info!("Model loaded successfully after {} attempts", i + 1);
                    let handle = ModelHandle {
                        model_id: model_id.to_string(),
                        loaded_at: SystemTime::now(),
                        context_size: self.model_config.ctx_window,
                    };
                    self.current_handle = Some(handle.clone());
                    return Ok(handle);
                }
            }
        }
        
        Err(Error::RuntimeError("Failed to load model - server did not start".into()))
    }
    
    async fn get_handle(&self) -> Option<ModelHandle> {
        self.current_handle.clone()
    }
    
    async fn generate(
        &self,
        handle: &ModelHandle,
        messages: Vec<Message>,
        params: GenerationParams,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamFrame>> + Send>>> {
        // Verify handle matches
        if self.current_handle.as_ref() != Some(handle) {
            return Err(Error::InvalidModel("Model handle does not match loaded model".into()));
        }
        
        let prompt = self.build_prompt(&messages);
        let request_id = params.request_id.clone();
        
        #[derive(Serialize)]
        struct CompletionRequest {
            prompt: String,
            n_predict: usize,
            temperature: f32,
            top_p: f32,
            top_k: i32,
            repeat_penalty: f32,
            stop: Vec<String>,
            stream: bool,
        }
        
        let request = CompletionRequest {
            prompt,
            n_predict: params.max_tokens,
            temperature: params.temperature,
            top_p: params.top_p,
            top_k: params.top_k,
            repeat_penalty: params.repeat_penalty,
            stop: params.stop_sequences.clone(),
            stream: true,
        };
        
        let url = format!("{}/completion", self.server_url);
        let template = self.template_config.clone();
        let stop_sequences = self.model_config.stop_sequences.clone();
        let eos_token = self.model_config.eos_token.clone();
        let model_id = self.model_config.id.clone();
        
        // Setup cancellation
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let active_reqs = self.active_requests.clone();
        let req_id = request_id.clone();
        
        // Register active request
        {
            let mut reqs = active_reqs.write().await;
            reqs.insert(req_id.clone(), cancel_tx);
        }
        
        let stream = async_stream::stream! {
            // Track cleanup
            let _cleanup = scopeguard::guard((), |_| {
                let reqs = active_reqs.clone();
                let id = req_id.clone();
                tokio::spawn(async move {
                    let mut r = reqs.write().await;
                    r.remove(&id);
                });
            });
            
            // Send start frame immediately
            yield Ok(StreamFrame::Start {
                id: request_id.clone(),
                model: model_id.clone(),
                role: Role::Assistant,
            });
            
            // Build streaming request
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(0)) // No timeout for SSE
                .build()
                .unwrap();
            
            let request_json = serde_json::to_string(&request).unwrap();
            
            // Make request with cancellation support
            let response_future = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .body(request_json)
                .send();
            
            // Race between response and cancellation
            let response = tokio::select! {
                resp = response_future => resp,
                _ = cancel_rx => {
                    yield Ok(StreamFrame::Error {
                        message: "Request cancelled".to_string(),
                    });
                    return;
                }
            };
            
            match response {
                Ok(response) => {
                    if response.status().is_success() {
                        use futures::StreamExt;
                        
                        let mut bytes_stream = response.bytes_stream();
                        let mut buffer = Vec::new();
                        let mut accumulated = String::new();
                        let mut token_count = 0;
                        let mut first_token_time: Option<std::time::Instant> = None;
                        
                        while let Some(chunk_result) = bytes_stream.next().await {
                            match chunk_result {
                                Ok(bytes) => {
                                    buffer.extend_from_slice(&bytes);
                                    
                                    // Process SSE lines immediately for low latency
                                    while let Some(newline_pos) = buffer.windows(2).position(|w| w == b"\n\n") {
                                        let event_bytes = buffer.drain(..newline_pos + 2).collect::<Vec<_>>();
                                        let event = String::from_utf8_lossy(&event_bytes);
                                        
                                        // Parse SSE event
                                        for line in event.lines() {
                                            if line.starts_with("data: ") {
                                                let data = &line[6..];
                                                
                                                // Try to parse the JSON payload
                                                #[derive(Deserialize, Debug)]
                                                struct StreamChunk {
                                                    content: String,
                                                    stop: bool,
                                                    #[serde(default)]
                                                    generation_settings: Option<serde_json::Value>,
                                                    #[serde(default)]
                                                    timings: Option<serde_json::Value>,
                                                }
                                                
                                                if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                                                    if !chunk.content.is_empty() {
                                                        // Track timing
                                                        if first_token_time.is_none() {
                                                            first_token_time = Some(std::time::Instant::now());
                                                            debug!("First token received after start");
                                                        }
                                                        
                                                        accumulated.push_str(&chunk.content);
                                                        token_count += 1;
                                                        
                                                        // Send content immediately without buffering
                                                        // Don't clean individual tokens - they may be partial
                                                        yield Ok(StreamFrame::Delta {
                                                            content: chunk.content,
                                                        });
                                                    }
                                                    
                                                    if chunk.stop {
                                                        debug!("Generation complete, {} tokens", token_count);
                                                        
                                                        // Final cleanup of accumulated content
                                                        let final_cleaned = TemplateEngine::clean_response(
                                                            &accumulated,
                                                            &template,
                                                            &stop_sequences,
                                                            &eos_token,
                                                        );
                                                        
                                                        // If we stripped something at the end, send correction
                                                        if final_cleaned.stopped_at.is_some() {
                                                            yield Ok(StreamFrame::Delta {
                                                                content: "\n".to_string(), // Ensure clean ending
                                                            });
                                                        }
                                                        
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Stream read error: {}", e);
                                    yield Ok(StreamFrame::Error {
                                        message: format!("Stream error: {}", e),
                                    });
                                    break;
                                }
                            }
                        }
                        
                        // Send done frame with usage stats
                        yield Ok(StreamFrame::Done {
                            finish_reason: FinishReason::Stop,
                            usage: Usage {
                                prompt_tokens: request.prompt.len() / 4, // Rough estimate
                                completion_tokens: token_count,
                                total_tokens: (request.prompt.len() / 4) + token_count,
                            },
                        });
                    } else {
                        yield Ok(StreamFrame::Error {
                            message: format!("Server error: {}", response.status()),
                        });
                    }
                }
                Err(e) => {
                    yield Ok(StreamFrame::Error {
                        message: format!("Request failed: {}", e),
                    });
                }
            }
        };
        
        Ok(Box::pin(stream))
    }
    
    async fn cancel(&self, request_id: &str) -> Result<()> {
        let mut reqs = self.active_requests.write().await;
        if let Some(cancel_tx) = reqs.remove(request_id) {
            let _ = cancel_tx.send(());
            info!("Cancelled request: {}", request_id);
        } else {
            warn!("No active request found for cancellation: {}", request_id);
        }
        Ok(())
    }
    
    async fn health(&self) -> Result<RuntimeHealth> {
        let url = format!("{}/health", self.server_url);
        
        let is_healthy = match self.client.get(&url).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        };
        
        let uptime = self.start_time.elapsed()
            .unwrap_or_default()
            .as_secs();
        
        let active_count = self.active_requests.read().await.len();
        
        Ok(RuntimeHealth {
            is_healthy,
            model_loaded: self.current_handle.clone(),
            active_requests: active_count,
            uptime_seconds: uptime,
        })
    }
    
    async fn unload(&mut self) -> Result<()> {
        self.current_handle = None;
        // Server stays running, just mark as unloaded
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<()> {
        if let Some(mut child) = self.server_process.take() {
            child.kill().await
                .map_err(|e| Error::RuntimeError(format!("Failed to stop server: {}", e)))?;
        }
        self.current_handle = None;
        Ok(())
    }
}