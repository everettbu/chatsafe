# AI Contributor Guidelines for ChatSafe

ChatSafe is a local-first, privacy-preserving chat assistant. We use Claude Code and other AI tools for coding help. To ensure consistency and safety, please follow these rules:

## 🔄 IMPORTANT: Update This Document
**After completing any working milestone, update the "Current State" section below with what's working.**

## Current State (Last Updated: 2025-09-26)

### ✅ Working Components
1. **Local Inference Engine**
   - llama.cpp built with Metal GPU support on macOS
   - **Model**: Llama-3.2-3B-Instruct Q4_K_M (2GB)
   - Model path: `~/.local/share/chatsafe/models/llama-3.2-3b-instruct-q4_k_m.gguf`
   - Context window: 8192 tokens
   - ~50-70 tokens/sec on M4 Mac

2. **HTTP API Server** 
   - Running on `http://127.0.0.1:8081` (localhost only)
   - OpenAI-compatible endpoint: `/v1/chat/completions`
   - SSE streaming support (defaults to on, use `"stream": false` for JSON)
   - Architecture: Rust server → llama-server subprocess → Model
   - Additional endpoints: `/healthz`, `/version`
   
3. **Chat Interface**
   - `./chat.sh` - Interactive chat script (requires `"stream": false`)
   - `./test_golden.sh` - Test suite for quality checks

4. **Working Test Commands**
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
  
### Configuration
- **Model**: Llama-3.2-3B-Instruct Q4_K_M (only model)
- **Template**: Llama-3 Instruct format with proper headers
- **Stop sequences**: `<|eot_id|>`, `<|end_of_text|>`, `<|start_header_id|>`
- **Defaults**:
  - temperature=0.6, top_p=0.9, top_k=40, repeat_penalty=1.15
  - max_tokens=256 (configurable per request)
- **Quality**: 
  - Coherent, concise responses
  - Accurate math and reasoning  
  - Good instruction following

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
