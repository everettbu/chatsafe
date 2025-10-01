use crate::{Runtime, RuntimeHealth, ModelHandle, template_engine::TemplateEngine};
use async_trait::async_trait;
use chatsafe_common::{Message, GenerationParams, Result, Error, StreamFrame, Role, FinishReason, Usage};
use chatsafe_config::{ModelConfig, TemplateConfig, RuntimeConfig};
use futures::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Stdio};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, oneshot};
use tokio::time::{sleep, Duration, timeout};
use tracing::{info, warn, debug, error};

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
    ) -> Result<Self> {
        let server_url = format!("http://127.0.0.1:{}", runtime_config.llama_server_port);
        
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| Error::RuntimeError(format!("Failed to create HTTP client: {}", e)))?;
        
        Ok(Self {
            model_path,
            model_config,
            template_config,
            runtime_config,
            server_process: None,
            client,
            server_url,
            current_handle: None,
            start_time: SystemTime::now(),
            active_requests: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
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
    
    /// Clean up any existing llama-server process
    async fn cleanup_existing_process(&mut self) -> Result<()> {
        // First, try to clean up our tracked process
        if let Some(mut child) = self.server_process.take() {
            info!("Cleaning up existing llama-server process");
            
            // Kill the process
            if let Err(e) = child.kill().await {
                debug!("Kill signal failed (process may already be dead): {}", e);
            }
            
            // Wait for it to exit
            match timeout(Duration::from_secs(2), child.wait()).await {
                Ok(Ok(status)) => {
                    info!("Previous llama-server exited with status: {:?}", status);
                }
                Ok(Err(e)) => {
                    warn!("Error waiting for process exit: {}", e);
                }
                Err(_) => {
                    warn!("Timeout waiting for process to exit");
                }
            }
        }
        
        // Also check for orphaned processes on our port using lsof
        self.kill_orphaned_processes().await?;
        
        Ok(())
    }
    
    /// Kill any orphaned llama-server processes on our port
    async fn kill_orphaned_processes(&self) -> Result<()> {
        let port = self.runtime_config.llama_server_port;
        
        // Use lsof to find processes listening on our port
        let output = Command::new("lsof")
            .args(&["-ti", &format!(":{}", port)])
            .output()
            .await;
        
        if let Ok(output) = output {
            if output.status.success() && !output.stdout.is_empty() {
                let pids = String::from_utf8_lossy(&output.stdout);
                for pid_str in pids.lines() {
                    if let Ok(pid) = pid_str.trim().parse::<i32>() {
                        warn!("Found orphaned process {} on port {}, killing it", pid, port);
                        
                        // Kill the process
                        let _ = Command::new("kill")
                            .args(&["-9", &pid.to_string()])
                            .output()
                            .await;
                    }
                }
                
                // Give processes time to die
                sleep(Duration::from_millis(200)).await;
            }
        }
        
        Ok(())
    }
    
    /// Check if the port is available
    async fn is_port_available(&self) -> bool {
        let port = self.runtime_config.llama_server_port;
        
        // Try to connect to the port
        match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            Ok(_) => {
                // Port is in use
                false
            }
            Err(_) => {
                // Port is available (connection refused)
                true
            }
        }
    }
    
    /// Wait for the server to become ready
    async fn wait_for_ready(&mut self) -> Result<()> {
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 60;
        
        loop {
            attempts += 1;
            
            if attempts > MAX_ATTEMPTS {
                return Err(Error::RuntimeError(
                    format!("Server failed to become ready after {} attempts", MAX_ATTEMPTS)
                ));
            }
            
            // Check if the process is still alive
            if let Some(child) = &mut self.server_process {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(Error::RuntimeError(
                        format!("llama-server process died with status: {:?}", status)
                    ));
                }
            }
            
            // Try health check
            if let Ok(health) = self.health().await {
                if health.is_healthy {
                    info!("Server ready after {} attempts", attempts);
                    return Ok(());
                }
            }
            
            sleep(Duration::from_millis(500)).await;
        }
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
        
        // Clean up any existing process first
        if let Err(e) = self.cleanup_existing_process().await {
            warn!("Error during cleanup: {}", e);
        }
        
        // Check if port is available before spawning
        if !self.is_port_available().await {
            return Err(Error::RuntimeError(format!(
                "Port {} is already in use. Another llama-server instance may be running.",
                self.runtime_config.llama_server_port
            )));
        }
        
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
           .stdout(Stdio::null())  // Discard stdout to prevent blocking
           .stderr(Stdio::null())  // Discard stderr to prevent blocking
           .kill_on_drop(true);   // Kill subprocess if parent dies
        
        let mut child = cmd.spawn()
            .map_err(|e| Error::RuntimeError(format!("Failed to start llama.cpp server: {}", e)))?;
        
        // Check if process started successfully
        sleep(Duration::from_millis(100)).await;
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(Error::RuntimeError(format!(
                    "llama-server exited immediately with status: {:?}. Check if binary exists at ./llama.cpp/build/bin/llama-server",
                    status
                )));
            }
            Ok(None) => {
                info!("llama-server process started successfully");
            }
            Err(e) => {
                return Err(Error::RuntimeError(format!("Failed to check process status: {}", e)));
            }
        }
        
        self.server_process = Some(child);
        
        // Wait for server to be ready with timeout
        let wait_result = timeout(Duration::from_secs(30), self.wait_for_ready()).await;
        
        match wait_result {
            Ok(Ok(())) => {
                info!("Model loaded successfully");
                let handle = ModelHandle {
                    model_id: Arc::from(model_id),
                    loaded_at: SystemTime::now(),
                    context_size: self.model_config.ctx_window,
                };
                self.current_handle = Some(handle.clone());
                Ok(handle)
            }
            Ok(Err(e)) => {
                // Clean up the failed process
                if let Err(cleanup_err) = self.cleanup_existing_process().await {
                    error!("Failed to cleanup after startup failure: {}", cleanup_err);
                }
                Err(e)
            }
            Err(_) => {
                // Timeout - clean up the process
                if let Err(cleanup_err) = self.cleanup_existing_process().await {
                    error!("Failed to cleanup after timeout: {}", cleanup_err);
                }
                Err(Error::RuntimeError("Timeout waiting for model to load".into()))
            }
        }
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
        // Use references where possible, Arc for values moved into async block
        let template = Arc::new(self.template_config.clone());
        let stop_sequences = Arc::new(self.model_config.stop_sequences.clone());
        let eos_token = Arc::new(self.model_config.eos_token.clone());
        let model_id = Arc::new(self.model_config.id.clone());
        let request_id_arc = Arc::new(request_id.clone());
        
        // Setup cancellation
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let active_reqs = self.active_requests.clone();
        
        // Register active request
        {
            let mut reqs = active_reqs.write().await;
            reqs.insert(request_id_arc.to_string(), cancel_tx);
        }
        
        let stream = async_stream::stream! {
            // Track cleanup
            let _cleanup = scopeguard::guard((), |_| {
                let reqs = active_reqs.clone();
                let id = request_id_arc.to_string();
                tokio::spawn(async move {
                    let mut r = reqs.write().await;
                    r.remove(&id);
                });
            });
            
            // Send start frame immediately
            yield Ok(StreamFrame::Start {
                id: request_id_arc.to_string(),
                model: model_id.to_string(),
                role: Role::Assistant,
            });
            
            // Build streaming request with no timeout for SSE
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(300)) // Long timeout for streaming
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    yield Ok(StreamFrame::Error {
                        message: format!("Failed to create HTTP client: {}", e),
                    });
                    return;
                }
            };
            
            let request_json = match serde_json::to_string(&request) {
                Ok(json) => json,
                Err(e) => {
                    yield Ok(StreamFrame::Error {
                        message: format!("Failed to serialize request: {}", e),
                    });
                    return;
                }
            };
            
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
                                                        
                                                        // Check if we've accumulated potential role pollution
                                                        // If so, replace the entire response
                                                        if accumulated.contains("AI:") && accumulated.contains("You:") {
                                                            // This is role pollution, send replacement message once
                                                            if !accumulated.contains("I understand you'd like me to respond") {
                                                                let replacement = "I understand you'd like me to respond, but I should avoid role-playing conversations. How can I help you directly?";
                                                                // Clear accumulated and replace with our message
                                                                accumulated = replacement.to_string();
                                                                yield Ok(StreamFrame::Delta {
                                                                    content: replacement.to_string(),
                                                                });
                                                                // Force stop
                                                                break;
                                                            }
                                                        } else {
                                                            // Normal content, send as-is
                                                            yield Ok(StreamFrame::Delta {
                                                                content: chunk.content,
                                                            });
                                                        }
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
            // First try graceful termination
            debug!("Sending termination signal to llama-server");
            
            // Send kill signal
            if let Err(e) = child.kill().await {
                warn!("Failed to send kill signal: {}", e);
            }
            
            // Wait for the process to actually exit (with timeout)
            match timeout(Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => {
                    info!("llama-server exited with status: {:?}", status);
                }
                Ok(Err(e)) => {
                    error!("Error waiting for process exit: {}", e);
                }
                Err(_) => {
                    error!("Timeout waiting for llama-server to exit, may have leaked process");
                }
            }
        }
        
        self.current_handle = None;
        
        // Double-check port is released
        sleep(Duration::from_millis(100)).await;
        if !self.is_port_available().await {
            warn!("Port {} still in use after shutdown", self.runtime_config.llama_server_port);
        }
        
        Ok(())
    }
}