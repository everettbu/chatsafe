# Error Semantics Guide

ChatSafe uses a unified error taxonomy defined in `crates/common/src/error.rs`. All errors map to appropriate HTTP status codes and provide clear, actionable messages.

## Error Types

### Client Errors (4xx)

| Error | HTTP Status | Description | Example |
|-------|-------------|-------------|---------|
| `InvalidRequest` | 400 | Malformed or invalid request | Missing required field, invalid JSON |
| `MissingMessages` | 400 | No messages in request | Empty messages array |
| `EmptyContent` | 400 | Message has no content | `{"role": "user", "content": ""}` |
| `InvalidRole` | 400 | Unknown or invalid role | Role not "user", "assistant", or "system" |
| `ContextOverflow` | 400 | Exceeds model context window | 10k tokens for 8k window |
| `InvalidParameter` | 400 | Invalid generation parameter | Temperature > 2.0 |
| `ModelNotFound` | 404 | Requested model doesn't exist | Unknown model ID |
| `UnsupportedMediaType` | 415 | Wrong content type | Not application/json |
| `TooManyRequests` | 429 | Rate limit exceeded | Too many concurrent requests |

### Server Errors (5xx)

| Error | HTTP Status | Description | Example |
|-------|-------------|-------------|---------|
| `RuntimeNotReady` | 503 | Runtime not initialized | Server starting up |
| `RuntimeError` | 500 | Generation/inference failure | Model crash, OOM |
| `ConfigError` | 500 | Configuration problem | Invalid registry |
| `ModelLoadError` | 500 | Failed to load model | File not found, corrupt |
| `TemplateError` | 500 | Template processing failed | Invalid template format |
| `InternalError` | 500 | Unexpected server error | Panic, unhandled case |

## Error Response Format

All errors return a consistent JSON structure:

```json
{
  "error": {
    "message": "Human-readable error description",
    "type": "error_type_snake_case",
    "details": {
      "field": "additional context"
    }
  }
}
```

### Examples

#### Invalid Request
```json
{
  "error": {
    "message": "Temperature must be between 0.0 and 2.0",
    "type": "invalid_parameter",
    "details": {
      "parameter": "temperature",
      "value": 3.5
    }
  }
}
```

#### Model Not Found
```json
{
  "error": {
    "message": "Model not found: gpt-4",
    "type": "model_not_found",
    "details": {
      "requested": "gpt-4",
      "available": ["llama-3.2-3b-instruct-q4_k_m", "mistral-7b-instruct-v0.3-q4"]
    }
  }
}
```

#### Runtime Error
```json
{
  "error": {
    "message": "Model generation failed",
    "type": "runtime_error",
    "details": {
      "reason": "Out of memory"
    }
  }
}
```

## Error Handling in Code

### Creating Errors

```rust
use chatsafe_common::{Error, ErrorResponse};

// Client error
let err = Error::InvalidParameter {
    parameter: "temperature".to_string(),
    message: "Must be between 0.0 and 2.0".to_string(),
};

// Convert to HTTP response
let response = ErrorResponse::from(&err);
let status = err.status_code(); // 400
```

### Request Validation

```rust
impl ChatCompletionRequest {
    pub fn validate(&self) -> Result<(), Error> {
        // Check required fields
        if self.messages.is_empty() {
            return Err(Error::MissingMessages);
        }
        
        // Validate parameters
        if let Some(temp) = self.temperature {
            if temp < 0.0 || temp > 2.0 {
                return Err(Error::InvalidParameter {
                    parameter: "temperature".to_string(),
                    message: format!("Value {} out of range", temp),
                });
            }
        }
        
        Ok(())
    }
}
```

### Error Propagation

```rust
// In handlers
async fn chat_completion(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    // Validate request
    if let Err(e) = request.validate() {
        return Err((e.status_code(), Json(ErrorResponse::from(&e))));
    }
    
    // Handle runtime errors
    let response = state.runtime
        .generate(&handle, messages, params)
        .await
        .map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, 
             Json(ErrorResponse::from(&e)))
        })?;
        
    Ok(response)
}
```

## Streaming Errors

Errors during SSE streaming are sent as data frames:

```
data: {"error": {"message": "Generation failed", "type": "runtime_error"}}
```

Then the stream terminates. Clients should:
1. Parse data frames for error objects
2. Stop processing on error
3. Display error message to user

## Client Implementation

### JavaScript/TypeScript
```javascript
const response = await fetch('/v1/chat/completions', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(request)
});

if (!response.ok) {
  const error = await response.json();
  console.error(`Error: ${error.error.message} (${error.error.type})`);
  return;
}

if (request.stream) {
  // Handle SSE stream
  const reader = response.body.getReader();
  // ... parse SSE, check for error frames
} else {
  const data = await response.json();
  // Use response
}
```

### Bash/curl
```bash
# Non-streaming
response=$(curl -s -w "\n%{http_code}" -X POST \
  http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages": [...]}')

http_code=$(echo "$response" | tail -n1)
body=$(echo "$response" | head -n-1)

if [ "$http_code" -ne 200 ]; then
  error_msg=$(echo "$body" | jq -r '.error.message')
  echo "Error: $error_msg" >&2
  exit 1
fi
```

## Privacy Considerations

Error messages are designed to be helpful without leaking sensitive information:

- ✅ No user prompts or responses in errors
- ✅ No file paths or system details exposed
- ✅ No stack traces in production
- ✅ Generic messages for security-sensitive failures

## Testing Error Handling

The test suite includes specific error cases:

```bash
# Test invalid request
./test_comprehensive.sh

# Specific error scenarios
- Empty messages array (400)
- Invalid temperature (400)
- Unknown model (404)
- Missing content-type (415)
```

## Common Issues

**"Runtime not ready" (503)**
- Server is starting up, model still loading
- Wait a few seconds and retry

**"Context overflow" (400)**
- Request exceeds model's context window
- Reduce message history or use larger model

**"Invalid parameter" (400)**
- Generation parameter out of valid range
- Check temperature (0-2), top_p (0-1), etc.

**"Model generation failed" (500)**
- Usually out of memory or model crash
- Check logs, restart server if needed