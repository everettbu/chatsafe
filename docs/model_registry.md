# Model Registry Configuration Guide

The model registry (`crates/config/src/default_registry.json`) is the single source of truth for model configuration in ChatSafe.

## Schema Fields

### Model Entry

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | ✓ | Unique identifier for the model (e.g., "llama-3.2-3b-instruct-q4_k_m") |
| `name` | string | ✓ | Human-readable display name |
| `file_name` | string | ✓ | GGUF filename in models directory |
| `size_gb` | number | ✓ | Model file size in gigabytes |
| `ctx_window` | number | ✓ | Maximum context window in tokens |
| `gpu_layers` | number | ✓ | Number of layers to offload to GPU (-1 for all) |
| `threads` | number | ✓ | CPU threads for inference |
| `batch_size` | number | ✓ | Batch size for processing |
| `template` | string | ✓ | Template format: "llama3", "chatml", "alpaca" |
| `stop_sequences` | array | ✓ | Token sequences that stop generation |
| `default` | boolean |  | Whether this is the default model |
| `defaults` | object |  | Default generation parameters |

### Defaults Object

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `temperature` | number | 0.7 | Randomness (0.0-2.0) |
| `max_tokens` | number | 2000 | Maximum response length |
| `top_p` | number | 0.9 | Nucleus sampling (0.0-1.0) |
| `top_k` | number | 40 | Top-k sampling |
| `repeat_penalty` | number | 1.1 | Repetition penalty |

## Template Formats

### Llama3 Template
```
<|begin_of_text|><|start_header_id|>system<|end_header_id|>
{system}<|eot_id|>
<|start_header_id|>user<|end_header_id|>
{user}<|eot_id|>
<|start_header_id|>assistant<|end_header_id|>
```

### ChatML Template
```
<|im_start|>system
{system}<|im_end|>
<|im_start|>user
{user}<|im_end|>
<|im_start|>assistant
```

### Alpaca Template
```
### Instruction:
{system}

### Input:
{user}

### Response:
```

## Example Configuration

```json
{
  "models": [
    {
      "id": "llama-3.2-3b-instruct-q4_k_m",
      "name": "Llama 3.2 3B Instruct (Q4)",
      "file_name": "llama-3.2-3b-instruct-q4_k_m.gguf",
      "size_gb": 2.0,
      "ctx_window": 8192,
      "gpu_layers": -1,
      "threads": 8,
      "batch_size": 512,
      "template": "llama3",
      "stop_sequences": [
        "<|eot_id|>",
        "<|end_of_text|>",
        "<|start_header_id|>"
      ],
      "default": true,
      "defaults": {
        "temperature": 0.7,
        "max_tokens": 2000,
        "top_p": 0.9,
        "top_k": 40,
        "repeat_penalty": 1.1
      }
    },
    {
      "id": "mistral-7b-instruct-v0.3-q4",
      "name": "Mistral 7B Instruct v0.3 (Q4)",
      "file_name": "mistral-7b-instruct-v0.3.Q4_K_M.gguf",
      "size_gb": 4.4,
      "ctx_window": 32768,
      "gpu_layers": 33,
      "threads": 8,
      "batch_size": 512,
      "template": "chatml",
      "stop_sequences": [
        "</s>",
        "<|im_end|>"
      ],
      "defaults": {
        "temperature": 0.7,
        "max_tokens": 4000,
        "top_p": 0.95
      }
    }
  ]
}
```

## Usage in Code

```rust
use chatsafe_config::{ModelRegistry, ModelConfig};

// Load the default registry
let registry = ModelRegistry::load_defaults()?;

// Get a specific model
let model = registry.get_model("llama-3.2-3b-instruct-q4_k_m")?;

// Get the default model
let default_model = registry.get_default_model()?;

// Apply parameter overrides
let params = registry.apply_overrides(
    "llama-3.2-3b-instruct-q4_k_m",
    Some(0.8),   // temperature override
    Some(1000),   // max_tokens override
    None,         // use default top_p
    None,         // use default top_k
    None          // use default repeat_penalty
)?;
```

## Adding a New Model

1. Download the GGUF file to `~/.local/share/chatsafe/models/`
2. Add an entry to `default_registry.json`
3. Ensure the template format is supported
4. Set appropriate stop sequences for clean output
5. Test with: `curl -X POST http://127.0.0.1:8081/v1/chat/completions -d '{"model": "your-model-id", "messages": [...]}'`

## Template Implementation

Templates are applied by the runtime's `TemplateEngine` (`crates/runtime/src/template.rs`):

- System prompts are optional and prepended
- Conversation history maintains role boundaries
- Stop sequences prevent instruction leakage
- Role markers are stripped from output

## Resource Requirements

| Model Size | RAM Required | GPU Memory | Typical Speed |
|------------|--------------|------------|---------------|
| 3B (Q4) | ~4 GB | ~2 GB | 50-70 tok/s |
| 7B (Q4) | ~8 GB | ~5 GB | 30-40 tok/s |
| 13B (Q4) | ~12 GB | ~8 GB | 15-25 tok/s |

## Troubleshooting

**Model loads but generates garbage**
- Wrong template format - check `template` field
- Missing stop sequences - model continues past response

**Model fails to load**
- File not found - check `file_name` and path
- Insufficient memory - reduce `gpu_layers` or use smaller quant

**Responses cut off early**
- `max_tokens` too low - increase in defaults
- Stop sequence false positive - review sequences

**Role pollution in output**
- Template markers leaking - check stop sequences
- Template mismatch - verify correct template for model family