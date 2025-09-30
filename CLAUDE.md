# AI Contributor Guidelines for ChatSafe

ChatSafe is a local-first, privacy-preserving chat assistant. We use Claude Code and other AI tools for coding help. To ensure consistency and safety, please follow these rules:

## 🔄 IMPORTANT: Maintaining CURRENT_STATE.md

**CURRENT_STATE.md is the changelog and issue tracker. Update it after EVERY work session.**

### How to Update CURRENT_STATE.md

1. **Add a new changelog entry** at the top with:
   - Date and brief summary of work
   - List what was implemented/fixed with ✅
   - List remaining issues discovered

2. **When fixing a previous issue**:
   - Add "(Fixed ✅)" to the issue in Open Issues
   - Include the fix in your changelog entry
   - The issue stays in Open Issues as a record

3. **Track test status** after changes

4. **Never delete entries** - only add or mark as fixed

Example changelog entry:
```markdown
## 2025-10-01: Fixed Rate Limiting
- ✅ Implemented rate limiting with token bucket algorithm
- ✅ Added configurable limits per endpoint
- ✅ Tests: 17/17 unit tests now passing
Issues remaining:
- Command Injection Risk (High Priority)
- Missing Backpressure (Medium Priority)  
- No Rate Limiting (Fixed ✅)
```

Example of marking issue as fixed:
```markdown
## Open Issues
### High Priority
- **No Rate Limiting** (Fixed ✅): Vulnerable to DoS attacks
- **Command Injection Risk**: Model paths passed directly to shell
```

## 📊 Current System State
See [CURRENT_STATE.md](./CURRENT_STATE.md) for changelog, working features, and issue tracking.

## Module Architecture

### Module Map & Contracts

| Module | Purpose | Public API | Dependencies | DoD |
|--------|---------|------------|--------------|-----|
| `crates/common` | Shared DTOs, errors, streaming contracts | `ChatCompletionRequest/Response`, `StreamFrame`, `Error`, `Metrics` | Only std types | All downstream crates compile; no cross-layer types |
| `crates/config` | Model registry & configuration | `ModelRegistry::load()`, `ModelConfig`, `AppConfig` | common | Registry drives all model behavior; no hardcoded params |
| `crates/runtime` | Model runtime with template engine | `RuntimeHandle::generate()`, `ModelHandle` | common, config | Templates applied correctly; stop sequences enforced |
| `crates/local-api` | HTTP API server (Axum) | POST `/v1/chat/completions`, GET `/healthz`, `/metrics` | common, config, runtime | OpenAI-compatible; SSE streaming; localhost-only |

### Contract Boundaries
- **DTOs**: All request/response types in `common` - no Axum/Tokio types leak out
- **Errors**: Unified error taxonomy in `common::Error` with HTTP status mapping
- **Streaming**: `StreamFrame` enum defines all possible stream events
- **Metrics**: Privacy-preserving counters only - no payloads, no PII

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
- Follow existing module boundaries
- Keep code changes minimal and testable
- Ensure all network endpoints bind only to `127.0.0.1`
- Use llama-server (HTTP) instead of llama-cli (pipes) to avoid deadlocks

## Testing
- After implementing a feature, provide:
  - Example usage (curl, CLI)
  - Expected output shape
  - Any known limitations
- Run test suites to verify:
  - `./test_golden.sh` - Core functionality
  - `./test_role_pollution.sh` - Template boundaries
  - `./test_comprehensive.sh` - Full integration

## Project Structure
```
/crates
  /common           # Shared DTOs, errors, streaming contracts
  /config           # Model registry & configuration
  /runtime          # Model runtime with template engine
  /local-api        # HTTP API server (Axum)
/docs               # Technical documentation
/llama.cpp          # Built inference engine
/models             # GGUF model files
CURRENT_STATE.md    # Current system state and progress
```

## ⚠️ Design Limitations

**These are architectural constraints and design decisions that would require significant refactoring to change.**

### Security Model
- **No Authentication**: Designed for local-only use
- **Trust Boundary**: Assumes trusted local environment

### Architecture Decisions
- **Subprocess Model**: Uses llama-server subprocess vs FFI
- **No Distributed Support**: Single-node only
- **Stateless Design**: No session management

### Resource Constraints
- **Single Model Loading**: One model active at a time
- **Context Window Fixed**: Set by model, not dynamic

**For bugs, implementation issues, and fixes, see [Open Issues in CURRENT_STATE.md](./CURRENT_STATE.md#open-issues).**

## Development Guidelines

### Privacy First
- No telemetry or outbound requests
- No PII in logs or metrics
- Localhost-only by default
- In-memory metrics only

### Code Quality
- Use existing patterns and conventions
- Prefer safe error handling over unwrap()
- Minimize cloning in hot paths
- Add tests for new functionality

## Related Documentation
- [Model Registry Guide](./docs/model_registry.md) - Configuration fields and examples
- [Error Semantics](./docs/errors.md) - Public error types and HTTP mappings

---

*Remember: Claude Code is a collaborator, not an autopilot. If in doubt, stop and ask.*
