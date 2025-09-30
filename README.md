# ChatSafe

A local-first, privacy-preserving chat assistant powered by open-source LLMs. ChatSafe runs entirely on your machine with no external API calls, telemetry, or data collection.

## Features

- 🔒 **100% Local** - All inference happens on your machine
- 🚀 **Fast** - Metal GPU acceleration on macOS (50-70 tokens/sec)
- 🌊 **Real-time Streaming** - Token-by-token SSE streaming
- 🔧 **OpenAI Compatible** - Drop-in replacement for OpenAI API
- 📊 **Privacy-First Metrics** - Optional observability without data leakage
- 🎯 **Production Ready** - Comprehensive test suite, proper error handling

## Quick Start

### Prerequisites

- macOS (Metal GPU support) or Linux
- Rust 1.70+ 
- 8GB+ RAM recommended
- 4GB disk space for models

### Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/chatsafe.git
cd chatsafe

# Build the project
cargo build --release

# Download the default model (2GB)
./scripts/download_models.sh

# Start the server
./target/release/chatsafe-server
```

### Basic Usage

```bash
# Chat via API
curl -X POST http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {"role": "user", "content": "Hello, how are you?"}
    ],
    "stream": false
  }'

# Interactive chat
./chat.sh
```

## Architecture

ChatSafe uses a clean modular architecture:

```
┌─────────────────┐
│   HTTP Client   │
└────────┬────────┘
         │ OpenAI API
┌────────▼────────┐
│   local-api     │  Axum HTTP Server
│   (Port 8081)   │  SSE Streaming
└────────┬────────┘
         │
┌────────▼────────┐
│    runtime      │  Template Engine
│                 │  Stop Sequences
└────────┬────────┘
         │
┌────────▼────────┐
│  llama-server   │  llama.cpp subprocess
│   subprocess    │  Metal/CUDA acceleration
└─────────────────┘
```

### Modules

| Module | Purpose | Key Files |
|--------|---------|-----------|
| `crates/common` | Shared types, DTOs, errors | `lib.rs`, `error.rs`, `metrics.rs` |
| `crates/config` | Model registry, configuration | `registry.rs`, `default_registry.json` |
| `crates/runtime` | LLM runtime, templating | `llama_adapter.rs`, `template.rs` |
| `crates/local-api` | HTTP server, API endpoints | `main.rs` |

## API Reference

### Chat Completion

**Endpoint:** `POST /v1/chat/completions`

```json
{
  "messages": [
    {"role": "system", "content": "You are a helpful assistant"},
    {"role": "user", "content": "Hello"}
  ],
  "model": "llama-3.2-3b-instruct-q4_k_m",
  "stream": true,
  "temperature": 0.7,
  "max_tokens": 2000
}
```

**Streaming Response (SSE):**
```
data: {"choices":[{"delta":{"content":"Hello"}}]}
data: {"choices":[{"delta":{"content":" there"}}]}
data: [DONE]
```

### Other Endpoints

- `GET /healthz` - Health check
- `GET /metrics` - Privacy-preserving metrics
- `GET /models` - List available models
- `GET /version` - API version

## Configuration

### Model Registry

Models are configured in `crates/config/src/default_registry.json`:

```json
{
  "models": [{
    "id": "llama-3.2-3b-instruct-q4_k_m",
    "name": "Llama 3.2 3B",
    "file_name": "llama-3.2-3b-instruct-q4_k_m.gguf",
    "ctx_window": 8192,
    "template": "llama3",
    "stop_sequences": ["<|eot_id|>"],
    "defaults": {
      "temperature": 0.7,
      "max_tokens": 2000
    }
  }]
}
```

### Server Configuration

Create `config.toml` (optional):

```toml
[server]
host = "127.0.0.1"
port = 8081

[runtime]
model_dir = "~/.local/share/chatsafe/models"
cache_dir = "~/.cache/chatsafe"
```

## Development

### Building from Source

```bash
# Development build
cargo build

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run --bin chatsafe-server
```

### Testing

```bash
# Run all tests
./run_tests.sh

# Run specific test category
./run_tests.sh --security
./run_tests.sh --quick
./run_tests.sh --unit

# Run individual test suites
./tests/test_comprehensive.sh
./tests/test_golden.sh
./tests/test_security.sh
```

See [tests/README.md](./tests/README.md) for detailed testing documentation.

### Project Structure

```
chatsafe/
├── crates/
│   ├── common/          # Shared types and contracts
│   ├── config/          # Configuration and model registry
│   ├── runtime/         # LLM runtime and templating
│   └── local-api/       # HTTP API server
├── docs/                # Technical documentation
│   ├── model_registry.md # Model configuration guide
│   ├── errors.md        # Error handling reference
│   └── test_coverage.md # Test gap analysis
├── tests/              # All test suites
│   ├── test_comprehensive.sh # Integration tests
│   ├── test_golden.sh       # Quality tests
│   ├── test_security.sh     # Security tests
│   └── README.md            # Testing guide
├── llama.cpp/          # Git submodule (inference engine)
├── run_tests.sh        # Main test runner
├── CLAUDE.md           # AI contributor guidelines
├── CURRENT_STATE.md    # Development changelog
└── README.md           # This file
```

## Contributing

### For Human Contributors

1. Read [CURRENT_STATE.md](./CURRENT_STATE.md) for current issues
2. Check [docs/](./docs/) for technical details
3. Follow existing code patterns
4. Add tests for new features
5. Update CURRENT_STATE.md after changes

### For AI Contributors

See [CLAUDE.md](./CLAUDE.md) for specific guidelines on:
- When to stop and ask for clarification
- How to update progress tracking
- Module boundaries and contracts
- Privacy-first principles

### Code Style

- Use `cargo fmt` before committing
- Follow Rust naming conventions
- Prefer `Result<T, Error>` over `unwrap()`
- Add doc comments for public APIs

## Privacy & Security

ChatSafe is designed with privacy as the top priority:

- ✅ **No telemetry** - Zero external API calls
- ✅ **No logging of prompts/responses** - Only metadata
- ✅ **Localhost only** - Binds to 127.0.0.1
- ✅ **No auth required** - Designed for local use
- ✅ **In-memory metrics** - No persistent storage

## Troubleshooting

### Server won't start

```bash
# Check if port is in use
lsof -i :8081

# Kill orphaned llama-server processes
pkill -f llama-server
```

### Model not loading

```bash
# Verify model exists
ls ~/.local/share/chatsafe/models/

# Check available memory
vm_stat | grep "Pages free"

# Try with fewer GPU layers
# Edit default_registry.json: "gpu_layers": 20
```

### Slow generation

- Ensure Metal/CUDA is enabled in build
- Check GPU utilization: `sudo powermetrics --samplers gpu_power`
- Reduce context window or batch size

## Supported Models

| Model | Size | Speed | Context | Status |
|-------|------|-------|---------|--------|
| Llama 3.2 3B (Q4_K_M) | 2GB | 50-70 tok/s | 8K | ✅ Active |

*Note: Multi-model support infrastructure is in place. Additional models can be added to the registry when model switching is implemented.*

## Performance

On M4 MacBook Pro:
- First token latency: ~200ms
- Streaming: 50-70 tokens/second
- Concurrent streams: 4+
- Memory usage: ~3GB with 3B model

## License

MIT - See [LICENSE](./LICENSE) file

## Acknowledgments

- [llama.cpp](https://github.com/ggerganov/llama.cpp) - Inference engine
- [Axum](https://github.com/tokio-rs/axum) - Web framework
- [Tokio](https://tokio.rs/) - Async runtime

## Status

See [CURRENT_STATE.md](./CURRENT_STATE.md) for:
- Current working features
- Known issues and bugs
- Recent improvements
- Development changelog

## Support

- Issues: [GitHub Issues](https://github.com/yourusername/chatsafe/issues)
- Documentation: [docs/](./docs/)
- Model Registry: [docs/model_registry.md](./docs/model_registry.md)
- Error Reference: [docs/errors.md](./docs/errors.md)