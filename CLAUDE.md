# AI Contributor Guidelines for ChatSafe

ChatSafe is a local-first, privacy-preserving chat assistant. We use Claude Code and other AI tools for coding help. To ensure consistency and safety, please follow these rules:

## 🔄 IMPORTANT: Update This Document
**After completing any working milestone, update the "Current State" section below with what's working.**

## Current State (Last Updated: 2025-09-30 - FULLY REFACTORED & WORKING)

### ✅ Clean Architecture Implemented
1. **Module Structure**
   - `crates/common` - Shared DTOs, errors, streaming contracts
   - `crates/config` - Model registry & configuration management
   - `crates/runtime` - Model runtime with template engine
   - `crates/local-api` - HTTP API server (Axum)
   - Clear module boundaries with no cross-layer leakage

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
   
3. **Model Registry (Single Source of Truth)**
   - JSON-based model configuration with templates
   - Support for multiple models (Llama, Mistral, Phi)
   - Per-model defaults (temperature, max_tokens, etc.)
   - Resource requirements tracking
   - Template system for different model families

4. **Chat Interface**
   - `./chat.sh` - Interactive chat script with multi-line support (detects and handles pasted content)
   - `./test_golden.sh` - Test suite for quality checks (✅ ALL PASSING)
   - `./test_role_pollution.sh` - Role boundary tests (✅ ALL PASSING)
   - Additional test scripts: `test_confusion.sh`, `test_dod.sh`, `test_overload.sh`

5. **Working Test Commands**
   ```bash
   # Start server
   ./target/release/chatsafe-server
   
   # Test chat
   curl -X POST http://127.0.0.1:8081/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"messages": [{"role": "user", "content": "Hello"}], "stream": false}'
   ```

### ✅ Issues Fixed Today
- ✅ Clean module architecture with proper boundaries
- ✅ Model registry as single source of truth
- ✅ Centralized template engine with role pollution prevention
- ✅ Proper stop sequence enforcement
- ✅ Request validation with bounds checking
- ✅ Error taxonomy with proper HTTP status codes
- ✅ All golden tests passing
- ✅ All role pollution tests passing

### ✅ Streaming Improvements (2025-09-30)
- **True incremental SSE streaming** - Token-by-token delivery 
- **Cancellation support** - Request cancellation propagates from HTTP to runtime
- **Backpressure handling** - Non-blocking stream processing
- **Parallel stream stability** - Supports 4+ concurrent streams
- **First token sub-second** - Low latency streaming response
- **Verification**: `curl -N` shows 50+ data frames per response

### 🛠️ Major Refactoring Completed (2025-09-30)
**Architecture Overhaul:**
- Implemented clean module boundaries with no cross-layer leakage
- Created shared contracts in `crates/common` with strict validation
- Built comprehensive model registry system
- Centralized templating and stop sequence handling in runtime
- Migrated local-api to use new architecture

**Fixes Applied:**
- Fixed role pollution (AI:/You:) in responses
- Improved template marker cleaning to prevent instruction leakage  
- Added prompt truncation to prevent context overflow
- Enhanced response cleaning to remove role labels only at line start
- Fixed chat.sh crash on special characters and multi-line input
- Added proper JSON escaping and error handling in chat.sh
  
### Configuration
- **Model Registry**: JSON-based configuration in `crates/config/src/default_registry.json`
- **Default Model**: Llama-3.2-3B-Instruct Q4_K_M
- **Available Models**: Llama 3.2 (3B), Mistral (7B), Phi-3 Mini
- **Template System**: Llama3, ChatML, Alpaca formats
- **Stop sequences**: Configured per model in registry
- **Defaults**: Configured per model with overrides supported
- **Quality**: 
  - ✅ All golden tests passing
  - ✅ No role pollution
  - ✅ Clean turn boundaries
  - ✅ Proper stop sequence enforcement

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

## Refactor Roadmap

### Suggested Refactor Milestones (Small, Sequential)

1. **Contracts pass**
   - Extract common DTOs/errors/traits to `crates/common`
   - DoD: API, runtime, and tests compile against shared types

2. **Model registry integration**
   - Load defaults/stops/templates from registry; delete hard-coded params
   - DoD: behavior unchanged; config drives choices

3. **Runtime seam clean-up**
   - Runtime owns templating + stops; local-api only maps HTTP↔DTO
   - DoD: role-pollution tests green

4. **Streaming tightening**
   - Incremental frames, flush policy, cancellation
   - DoD: granular `curl -N` ✅; concurrency stable

5. **Observability + security checks**
   - Add counters; ensure loopback-only and no egress in tests
   - DoD: privacy posture verified by tests

6. **Docs & CI polish**
   - Update contributor/docs; ensure CI runs golden suite
   - DoD: green pipeline; docs match reality

### Exit Criteria (Refactor Complete)
- Clear module boundaries; no cross-layer leakage
- Single sources of truth (registry for models; runtime for templates/stops)
- Granular SSE with robust cancellation; concurrency proven
- Tests lock contracts and golden behaviors; CI green
- Privacy/security posture unchanged (localhost-only, no egress)
- Docs equip new contributors (human/AI) to proceed without ambiguity

---

*Remember: Claude Code is a collaborator, not an autopilot. If in doubt, stop and ask.*
