use crate::{Runtime, RuntimeHealth, ModelHandle, template_engine::TemplateEngine};
use async_trait::async_trait;
use chatsafe_common::{Message, GenerationParams, Result, Error, StreamFrame, Role, FinishReason, Usage};
use chatsafe_config::{ModelConfig, TemplateConfig, RuntimeConfig};
use futures::Stream;
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::process::Command;
use tokio::sync::{RwLock, oneshot};
use tokio::time::{sleep, Duration, timeout};
use tracing::{info, warn};

// Constants
const HTTP_TIMEOUT_SECS: u64 = 300;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 5;
const HEALTH_CHECK_TIMEOUT_SECS: u64 = 2;
const HEALTH_CHECK_CONNECT_TIMEOUT_MS: u64 = 500;
const PROCESS_KILL_WAIT_MS: u64 = 200;
const SERVER_READY_MAX_ATTEMPTS: u32 = 60;
const SERVER_READY_CHECK_INTERVAL_MS: u64 = 500;
const PROCESS_START_WAIT_MS: u64 = 100;
const MODEL_LOAD_TIMEOUT_SECS: u64 = 30;
const DEFAULT_PARALLEL_REQUESTS: &str = "4";
const DEFAULT_N_PREDICT: &str = "-1";
const LLAMA_SERVER_BINARY: &str = "./llama.cpp/build/bin/llama-server";
const TOKEN_ESTIMATION_DIVISOR: usize = 4;
const KILL_SIGNAL: &str = "-9";

/// Adapter for llama.cpp server
pub struct LlamaAdapter {
    model_path: PathBuf,
    model_config: ModelConfig,
    template_config: TemplateConfig,
    runtime_config: RuntimeConfig,
    process_manager: crate::process_manager::ProcessManager,
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
        
        Ok(Self {
            model_path,
            model_config,
            template_config,
            runtime_config,
            process_manager: crate::process_manager::ProcessManager::new("llama-server".to_string()),
            server_url,
            current_handle: None,
            start_time: SystemTime::now(),
            active_requests: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }
    
    /// Create a default HTTP client with standard timeouts
    fn create_default_client() -> Result<Client> {
        Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
            .build()
            .map_err(|e| Error::RuntimeError(format!("Failed to create HTTP client: {}", e)))
    }
    
    /// Create a health check client with short timeouts
    fn create_health_check_client() -> Result<Client> {
        Client::builder()
            .timeout(Duration::from_secs(HEALTH_CHECK_TIMEOUT_SECS))
            .connect_timeout(Duration::from_millis(HEALTH_CHECK_CONNECT_TIMEOUT_MS))
            .build()
            .map_err(|e| Error::RuntimeError(format!("Failed to create health check client: {}", e)))
    }
    
    fn build_prompt(&self, messages: &[Message]) -> String {
        TemplateEngine::format_prompt(messages, &self.template_config)
    }
    
    /// Clean up any existing llama-server process
    async fn cleanup_existing_process(&mut self) -> Result<()> {
        // Use ProcessManager to clean up any tracked process
        self.process_manager.cleanup().await?;
        
        // Also check for orphaned processes on our port using lsof
        self.kill_orphaned_processes().await?;
        
        Ok(())
    }
    
    /// Kill any orphaned llama-server processes on our port
    async fn kill_orphaned_processes(&self) -> Result<()> {
        let port = self.runtime_config.llama_server_port;
        
        // Use lsof to find processes listening on our port
        let output = Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .await;
        
        if let Ok(output) = output {
            if output.status.success() && !output.stdout.is_empty() {
                let pids = String::from_utf8_lossy(&output.stdout);
                for pid_str in pids.lines() {
                    if let Ok(pid) = pid_str.trim().parse::<i32>() {
                        warn!("Found orphaned process {} on port {}, killing it", pid, port);
                        
                        // Kill the process (TODO: try SIGTERM first, then SIGKILL)
                        let _ = Command::new("kill")
                            .args([KILL_SIGNAL, &pid.to_string()])
                            .output()
                            .await;
                    }
                }
                
                // Give processes time to die
                sleep(Duration::from_millis(PROCESS_KILL_WAIT_MS)).await;
            }
        }
        
        Ok(())
    }
    
    /// Check if the port is available
    async fn is_port_available(&self) -> bool {
        let port = self.runtime_config.llama_server_port;
        
        // Try to connect to the port
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_err()
    }
    
    /// Wait for the server to become ready
    async fn wait_for_ready(&mut self) -> Result<()> {
        for attempts in 1..=SERVER_READY_MAX_ATTEMPTS {
            // Check if the process is still alive
            if !self.process_manager.is_running() {
                return Err(Error::RuntimeError(
                    "llama-server process died unexpectedly".to_string()
                ));
            }
            
            // Try health check
            if let Ok(health) = self.health().await {
                if health.is_healthy {
                    info!("Server ready after {} attempts", attempts);
                    return Ok(());
                }
            }
            
            sleep(Duration::from_millis(SERVER_READY_CHECK_INTERVAL_MS)).await;
        }
        
        Err(Error::RuntimeError(
            format!("Server failed to become ready after {} attempts", SERVER_READY_MAX_ATTEMPTS)
        ))
    }
    
    /// Build the llama-server command with all arguments
    fn build_server_command(&self) -> Command {
        let mut cmd = Command::new(LLAMA_SERVER_BINARY);
        cmd.arg("--model").arg(&self.model_path)
           .arg("--ctx-size").arg(self.model_config.ctx_window.to_string())
           .arg("--n-gpu-layers").arg(self.model_config.resources.gpu_layers.to_string())
           .arg("--host").arg("127.0.0.1")
           .arg("--port").arg(self.runtime_config.llama_server_port.to_string())
           .arg("--threads").arg(self.model_config.resources.threads.to_string())
           .arg("--n-predict").arg(DEFAULT_N_PREDICT)
           .arg("--parallel").arg(DEFAULT_PARALLEL_REQUESTS)
           .arg("--cont-batching")
           .arg("--flash-attn").arg("on");
        cmd
    }
    
    /// Handle cleanup after startup failure
    async fn cleanup_after_failure(&mut self, context: &str) {
        if let Err(e) = self.cleanup_existing_process().await {
            warn!("Failed to cleanup after {}: {}", context, e);
        }
    }
    
    /// Process SSE chunk and extract content
    fn parse_sse_chunk(data: &str) -> Result<StreamChunk> {
        serde_json::from_str::<StreamChunk>(data)
            .map_err(|e| Error::RuntimeError(format!("Failed to parse SSE chunk: {}. Data: {:?}", e, data)))
    }
    
    /// Clean streaming content by removing markers
    fn clean_streaming_content(content: &str) -> String {
        let markers = ["<|eot_id|>", "<|end_of_text|>", "<|start_header_id|>", "<|end_header_id|>"];
        let mut cleaned = content.to_string();
        for marker in &markers {
            cleaned = cleaned.replace(marker, "");
        }
        cleaned
    }
    
    /// Check for role pollution in accumulated content
    fn has_role_pollution(content: &str) -> bool {
        content.contains("AI:") && content.contains("You:")
    }
    
    /// Estimate token count from text
    fn estimate_tokens(text: &str) -> usize {
        text.len() / TOKEN_ESTIMATION_DIVISOR
    }
}

/// SSE stream chunk structure
#[derive(Deserialize, Debug)]
struct StreamChunk {
    content: String,
    stop: bool,
}

/// Completion request for llama.cpp server
#[derive(serde::Serialize)]
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
        
        // Start llama.cpp server using ProcessManager
        let cmd = self.build_server_command();
        
        // Spawn with proper stdout/stderr draining
        self.process_manager.spawn(cmd).await
            .map_err(|e| Error::RuntimeError(format!("Failed to start llama.cpp server: {}", e)))?;
        
        // Check if process started successfully
        sleep(Duration::from_millis(PROCESS_START_WAIT_MS)).await;
        if !self.process_manager.is_running() {
            return Err(Error::RuntimeError(
                format!("llama-server exited immediately. Check if binary exists at {}", LLAMA_SERVER_BINARY)
            ));
        }
        
        info!("llama-server process started successfully");
        
        // Wait for server to be ready with timeout
        let wait_result = timeout(
            Duration::from_secs(MODEL_LOAD_TIMEOUT_SECS), 
            self.wait_for_ready()
        ).await;
        
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
                self.cleanup_after_failure("startup failure").await;
                Err(e)
            }
            Err(_) => {
                self.cleanup_after_failure("timeout").await;
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
        // Use Arc for values moved into async block
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
        
        let stream = Self::create_generation_stream(StreamParams {
            request,
            url,
            request_id: request_id_arc,
            model_id,
            template,
            stop_sequences,
            eos_token,
            active_reqs,
            cancel_rx,
        });
        
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
        let health_client = Self::create_health_check_client()?;
        
        let is_healthy = match health_client.get(&url).send().await {
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
        // Use ProcessManager for proper cleanup
        self.process_manager.terminate().await?;
        
        self.current_handle = None;
        
        // Double-check port is released
        sleep(Duration::from_millis(PROCESS_START_WAIT_MS)).await;
        if !self.is_port_available().await {
            warn!("Port {} still in use after shutdown", self.runtime_config.llama_server_port);
        }
        
        Ok(())
    }
}

/// Parameters for stream generation
struct StreamParams {
    request: CompletionRequest,
    url: String,
    request_id: Arc<String>,
    model_id: Arc<String>,
    template: Arc<TemplateConfig>,
    stop_sequences: Arc<Vec<String>>,
    eos_token: Arc<String>,
    active_reqs: Arc<RwLock<std::collections::HashMap<String, oneshot::Sender<()>>>>,
    cancel_rx: oneshot::Receiver<()>,
}

impl LlamaAdapter {
    /// Create the generation stream
    fn create_generation_stream(params: StreamParams) -> impl Stream<Item = Result<StreamFrame>> + Send {
        async_stream::stream! {
            // Track cleanup
            let _cleanup = scopeguard::guard(params.active_reqs.clone(), |reqs| {
                let id = params.request_id.to_string();
                tokio::spawn(async move {
                    let mut r = reqs.write().await;
                    r.remove(&id);
                });
            });
            
            // Send start frame immediately
            yield Ok(StreamFrame::Start {
                id: params.request_id.to_string(),
                model: params.model_id.to_string(),
                role: Role::Assistant,
            });
            
            // Process the streaming response
            match Self::process_stream_response(
                params.request,
                params.url,
                params.template,
                params.stop_sequences,
                params.eos_token,
                params.cancel_rx,
            ).await {
                Ok(result) => {
                    // Yield all frames from the processing
                    for frame in result {
                        yield Ok(frame);
                    }
                }
                Err(e) => {
                    yield Ok(StreamFrame::Error {
                        message: e.to_string(),
                    });
                }
            }
        }
    }
    
    /// Process the streaming response from llama.cpp server
    async fn process_stream_response(
        request: CompletionRequest,
        url: String,
        template: Arc<TemplateConfig>,
        stop_sequences: Arc<Vec<String>>,
        eos_token: Arc<String>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) -> Result<Vec<StreamFrame>> {
        let mut frames = Vec::new();
        
        // Build streaming request
        let client = Self::create_default_client()?;
        
        let request_json = serde_json::to_string(&request)
            .map_err(|e| Error::RuntimeError(format!("Failed to serialize request: {}", e)))?;
        
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
            _ = &mut cancel_rx => {
                return Ok(vec![StreamFrame::Error {
                    message: "Request cancelled".to_string(),
                }]);
            }
        };
        
        let response = response
            .map_err(|e| Error::RuntimeError(format!("Request failed: {}", e)))?;
        
        if !response.status().is_success() {
            return Ok(vec![StreamFrame::Error {
                message: format!("Server error: {}", response.status()),
            }]);
        }
        
        // Process SSE stream
        frames.extend(
            Self::process_sse_stream(response, template, stop_sequences, eos_token, request.prompt).await?
        );
        
        Ok(frames)
    }
    
    /// Process SSE event stream
    async fn process_sse_stream(
        response: reqwest::Response,
        template: Arc<TemplateConfig>,
        stop_sequences: Arc<Vec<String>>,
        eos_token: Arc<String>,
        prompt: String,
    ) -> Result<Vec<StreamFrame>> {
        use futures::StreamExt;
        
        let mut frames = Vec::new();
        let mut bytes_stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut accumulated = String::new();
        let mut token_count = 0;
        let mut dropped_frames = 0;
        
        while let Some(chunk_result) = bytes_stream.next().await {
            let bytes = chunk_result
                .map_err(|e| Error::RuntimeError(format!("Stream error: {}", e)))?;
            
            buffer.extend_from_slice(&bytes);
            
            // Process SSE lines
            while let Some(newline_pos) = buffer.windows(2).position(|w| w == b"\n\n") {
                let event_bytes = buffer.drain(..newline_pos + 2).collect::<Vec<_>>();
                let event = String::from_utf8_lossy(&event_bytes);
                
                // Parse SSE event
                for line in event.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        match Self::parse_sse_chunk(data) {
                            Ok(chunk) => {
                                if !chunk.content.is_empty() {
                                accumulated.push_str(&chunk.content);
                                token_count += 1;
                                
                                // Check for role pollution
                                if Self::has_role_pollution(&accumulated) {
                                    if !accumulated.contains("I understand you'd like me to respond") {
                                        frames.push(StreamFrame::Delta {
                                            content: "I understand you'd like me to respond, but I should avoid role-playing conversations. How can I help you directly?".to_string(),
                                        });
                                        break;
                                    }
                                } else {
                                    // Clean and send the chunk
                                    let cleaned = Self::clean_streaming_content(&chunk.content);
                                    if !cleaned.trim().is_empty() {
                                        frames.push(StreamFrame::Delta {
                                            content: cleaned,
                                        });
                                    }
                                }
                                }
                                
                                if chunk.stop {
                                    // Final cleanup
                                    let final_cleaned = TemplateEngine::clean_response(
                                        &accumulated,
                                        &template,
                                        &stop_sequences,
                                        &eos_token,
                                    );
                                    
                                    if final_cleaned.stopped_at.is_some() {
                                        frames.push(StreamFrame::Delta {
                                            content: "\n".to_string(),
                                        });
                                    }
                                    
                                    break;
                                }
                            }
                            Err(e) => {
                                // Log the malformed frame but continue processing
                                warn!("Dropped malformed SSE frame: {}", e);
                                dropped_frames += 1;
                            }
                        }
                    }
                }
            }
        }
        
        // Log warning if any frames were dropped
        if dropped_frames > 0 {
            warn!("Dropped {} malformed SSE frames during streaming", dropped_frames);
        }
        
        // Send done frame with usage stats
        frames.push(StreamFrame::Done {
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: Self::estimate_tokens(&prompt),
                completion_tokens: token_count,
                total_tokens: Self::estimate_tokens(&prompt) + token_count,
            },
        });
        
        Ok(frames)
    }
}