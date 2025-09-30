# ChatSafe Test Suite

This directory contains all test suites for ChatSafe. Tests are organized by type and purpose.

## Quick Start

```bash
# Run all tests
../run_tests.sh

# Run specific test suite
./test_golden.sh
./test_security.sh
./test_comprehensive.sh

# Run only unit tests
cargo test
```

## Test Organization

### Core Test Suites

| Test Suite | Purpose | Runtime | Coverage |
|------------|---------|---------|----------|
| `test_golden.sh` | Core functionality validation | ~5s | Happy path |
| `test_comprehensive.sh` | Full integration testing | ~10s | All endpoints |
| `test_security.sh` | Security vulnerability testing | ~5s | Injection, traversal |
| `test_role_pollution.sh` | Template boundary testing | ~3s | Role separation |

### Specialized Tests

| Test Suite | Purpose | When to Run |
|------------|---------|-------------|
| `test_confusion.sh` | Edge cases and ambiguity | Before release |
| `test_dod.sh` | Definition of Done checks | After features |
| `test_overload.sh` | Load and stress testing | Performance validation |

### Unit Tests

Located in `crates/*/src/tests.rs` files:
- `common` - DTO validation, error handling
- `config` - Model registry, configuration
- `runtime` - Template engine, streaming
- `local-api` - (TODO) HTTP handlers

## Test Categories

### 1. Security Tests (`test_security.sh`)
- Command injection prevention
- Path traversal protection
- Input validation
- Resource exhaustion handling
- JSON/Header injection

### 2. Integration Tests (`test_comprehensive.sh`)
- Streaming vs non-streaming
- All HTTP endpoints
- Parameter validation
- Concurrent requests
- Error responses

### 3. Quality Tests (`test_golden.sh`)
- Response quality
- Instruction following
- Output formatting
- Token limits

### 4. Template Tests (`test_role_pollution.sh`)
- Role boundary enforcement
- Template marker cleaning
- Stop sequence handling
- System prompt isolation

## Running Tests

### Prerequisites
1. Server must be running: `./target/release/chatsafe-server`
2. Port 8081 must be available
3. Model must be loaded

### Full Test Run
```bash
# From project root
./run_tests.sh
```

### Individual Suites
```bash
# Security only
./tests/test_security.sh

# Quick validation
./tests/test_golden.sh

# Comprehensive
./tests/test_comprehensive.sh
```

### CI/CD Integration
```bash
# Start server in background
./target/release/chatsafe-server &
SERVER_PID=$!

# Run tests
./run_tests.sh

# Cleanup
kill $SERVER_PID
```

## Writing New Tests

### Shell Test Template
```bash
#!/bin/bash
test_my_feature() {
    response=$(curl -s -X POST http://127.0.0.1:8081/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "test"}]}')
    
    # Validate response
    if [[ "$response" == *"expected"* ]]; then
        return 0
    fi
    return 1
}

run_test "My feature test" test_my_feature
```

### Unit Test Template
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_my_function() {
        let result = my_function();
        assert_eq!(result, expected);
    }
}
```

## Test Coverage

For detailed coverage analysis and gaps, see: [docs/test_coverage.md](../docs/test_coverage.md)

### Current Coverage
- **Integration**: ~60% coverage
- **Unit Tests**: 19 tests across 3 modules
- **Security**: Basic coverage (new)
- **Performance**: Limited coverage

### Critical Gaps
1. No unit tests for `local-api` module
2. Limited error recovery testing
3. No resource leak detection
4. Missing edge case coverage

## Continuous Improvement

Tests should be added for:
- Every bug fix (regression test)
- Every new feature
- Every security concern
- Every performance issue

## Test Standards

1. **Fast**: Each test < 1 second
2. **Isolated**: No test dependencies
3. **Deterministic**: Same result every run
4. **Clear**: Descriptive names and output
5. **Maintainable**: Easy to update

## Troubleshooting

### Tests fail with "Server not running"
```bash
# Start the server first
./target/release/chatsafe-server
```

### Tests hang or timeout
```bash
# Check for zombie processes
pkill -f llama-server
pkill -f chatsafe-server
```

### Inconsistent results
- Check model is loaded correctly
- Verify no other services on port 8081
- Clear any test artifacts in /tmp