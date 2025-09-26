# AI Contributor Guidelines for ChatSafe

ChatSafe is a local-first, privacy-preserving chat assistant. We use Claude Code and other AI tools for coding help. To ensure consistency and safety, please follow these rules:

## 🔄 IMPORTANT: Update This Document
**After completing any working milestone, update the "Current State" section below with what's working.**

## Current State (Last Updated: 2025-09-26)

### ✅ Working Components
1. **Local Inference Engine**
   - llama.cpp built with Metal GPU support on macOS
   - **UPGRADED**: Llama-3.2-3B-Instruct Q4_K_M (replacing TinyLlama)
   - Model path: `~/.local/share/chatsafe/models/llama-3.2-3b-instruct-q4_k_m.gguf`
   - Context window: 8192 tokens

2. **HTTP API Server** 
   - Running on `http://127.0.0.1:8081` (localhost only)
   - OpenAI-compatible endpoint: `/v1/chat/completions`
   - SSE streaming support
   - Architecture: Rust server → llama-server subprocess → Model
   - New endpoint: `/version` (returns model info)
   
3. **Llama-3 Chat Template**
   - Proper Llama-3 Instruct formatting with headers
   - Stop sequences: `<|eot_id|>`, `<|end_of_text|>`, `<|start_header_id|>`
   - Default system prompt for conciseness

3. **Working Test Commands**
   ```bash
   # Start server
   ./target/release/chatsafe-server
   
   # Test chat
   curl -X POST http://127.0.0.1:8081/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"messages": [{"role": "user", "content": "Hello"}], "stream": false}'
   ```

### 🚧 Known Issues
- Streaming returns full response in one chunk (not token-by-token yet)
- Model download required on first run (~2GB)
  
### Model Upgrade Complete
- **Replaced TinyLlama 1.1B → Llama-3.2-3B-Instruct Q4_K_M**
- Optimized parameters for Llama-3:
  - temperature=0.6, top_p=0.9, top_k=40, repeat_penalty=1.15
  - max_tokens=256 (default, configurable per request)
- Expected quality improvements:
  - Coherent, concise responses
  - Accurate math and reasoning
  - Better instruction following

## Scope
- Focus on **incremental tasks** (well-defined changes or modules).
- Avoid trying to "complete the whole system" in one pass.

## When to Stop
- If you are **unsure about a design choice** (e.g., FFI vs subprocess, API shape).
- If integration details are **ambiguous** (e.g., error handling strategy not clear).
- If you detect a **blocking issue** (deadlocks, hangs, unexpected output).
- If you realize a **better approach exists** than the one you're generating.

When any of these happen, **STOP and report the snag** instead of continuing with assumptions.

## How to Report a Snag
- Clearly state: **"⚠️ Snag detected"**
- Clearly state **what is working**
- Describe the uncertainty or failure (e.g., "stdout not drained may cause deadlock").
- Suggest at most 2–3 paths forward, but do not decide on behalf of the human.

## Best Practices
- Follow existing module boundaries (`crates/infer-runtime`, `crates/local-api`, etc).
- Keep code changes minimal and testable.
- Ensure all network endpoints bind only to `127.0.0.1`.
- Default to **privacy-first**: no telemetry, no outbound requests.
- Use llama-server (HTTP) instead of llama-cli (pipes) to avoid deadlocks.

## Testing
- After implementing a feature, provide:
  - Example usage (curl, CLI).
  - Expected output shape.
  - Any known limitations.

## Project Structure
```
/crates
  /infer-runtime    # Manages llama.cpp server subprocess
  /local-api        # HTTP API server (Axum)
/llama.cpp          # Built inference engine
/models             # GGUF model files
```

---

*Remember: Claude Code is a collaborator, not an autopilot. If in doubt, stop and ask.*
