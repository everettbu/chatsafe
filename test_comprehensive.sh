#!/bin/bash
# Comprehensive integration test suite for ChatSafe
# Tests both streaming and non-streaming with golden outputs

set -e

API_URL="http://127.0.0.1:8081/v1/chat/completions"
METRICS_URL="http://127.0.0.1:8081/metrics"
MODELS_URL="http://127.0.0.1:8081/models"
VERSION_URL="http://127.0.0.1:8081/version"
HEALTH_URL="http://127.0.0.1:8081/healthz"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Test helper functions
run_test() {
    local test_name="$1"
    local test_func="$2"
    
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    echo -n "Running $test_name... "
    
    if $test_func > /dev/null 2>&1; then
        echo -e "${GREEN}✓${NC}"
        PASSED_TESTS=$((PASSED_TESTS + 1))
    else
        echo -e "${RED}✗${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        echo "  Failed: $test_name"
    fi
}

# Golden tests - Non-streaming
test_greeting_non_stream() {
    response=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Hello!"}], "stream": false}' | \
        jq -r '.choices[0].message.content')
    
    # Should be short, no role labels
    if [[ ${#response} -gt 100 ]]; then
        return 1
    fi
    if [[ "$response" == *"AI:"* ]] || [[ "$response" == *"You:"* ]]; then
        return 1
    fi
    return 0
}

test_math_non_stream() {
    response=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "What is 2+2?"}], "stream": false}' | \
        jq -r '.choices[0].message.content')
    
    # Should contain "4"
    if [[ "$response" == *"4"* ]]; then
        return 0
    fi
    return 1
}

test_summarize_non_stream() {
    response=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Summarize in one sentence: The quick brown fox jumps over the lazy dog. This pangram contains all letters of the alphabet."}], "stream": false}' | \
        jq -r '.choices[0].message.content')
    
    # Should be concise (under 200 chars) and clean
    if [[ ${#response} -lt 200 ]] && [[ "$response" != *"<|"* ]]; then
        return 0
    fi
    return 1
}

test_story_non_stream() {
    response=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Tell a 2-sentence story about a cat."}], "stream": false, "max_tokens": 100}' | \
        jq -r '.choices[0].message.content')
    
    # Should have multiple sentences
    sentence_count=$(echo "$response" | grep -o '[.!?]' | wc -l)
    if [ $sentence_count -ge 2 ]; then
        return 0
    fi
    return 1
}

# Golden tests - Streaming
test_greeting_stream() {
    # Count SSE data frames
    frames=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Hi"}], "stream": true}' | \
        grep -c "^data: {")
    
    # Should have multiple frames but not too many for a short response
    if [ $frames -gt 2 ] && [ $frames -lt 50 ]; then
        return 0
    fi
    return 1
}

test_stream_sequence() {
    # Check proper SSE sequence: Start → Delta* → Done
    response=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Say test"}], "stream": true}')
    
    # Check for role in first chunk
    has_role=$(echo "$response" | head -1 | grep -c '"role":"assistant"')
    # Check for [DONE] marker
    has_done=$(echo "$response" | grep -c "data: \[DONE\]")
    
    if [ $has_role -eq 1 ] && [ $has_done -eq 1 ]; then
        return 0
    fi
    return 1
}

# Contract tests
test_parameter_bounds() {
    # Test temperature bounds
    error=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "test"}], "temperature": 3.0}' | \
        jq -r '.error.message')
    
    if [[ "$error" == *"Temperature must be between"* ]]; then
        return 0
    fi
    return 1
}

test_empty_messages() {
    # Test empty messages array
    error=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [], "stream": false}' | \
        jq -r '.error.message')
    
    if [[ "$error" == *"Messages array cannot be empty"* ]]; then
        return 0
    fi
    return 1
}

test_max_tokens() {
    # Test max_tokens bounds
    error=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "hi"}], "max_tokens": 10000}' | \
        jq -r '.error.message')
    
    if [[ "$error" == *"max_tokens must be between"* ]]; then
        return 0
    fi
    return 1
}

# Role pollution tests
test_no_role_pollution() {
    # Test that role markers don't appear at line start
    response=$(curl -s -X POST $API_URL \
        -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Respond with multiple lines"}], "stream": false}' | \
        jq -r '.choices[0].message.content')
    
    # Check for role pollution patterns at line start
    if echo "$response" | grep -E '^(AI:|You:|User:|Assistant:|Human:|Bot:)' > /dev/null; then
        return 1
    fi
    return 0
}

# Endpoint tests
test_health_endpoint() {
    status=$(curl -s $HEALTH_URL | jq -r '.status')
    if [[ "$status" == "healthy" ]]; then
        return 0
    fi
    return 1
}

test_metrics_endpoint() {
    # Check metrics has expected fields
    metrics=$(curl -s $METRICS_URL)
    
    # Check for key metrics
    if echo "$metrics" | jq -e '.total_requests' > /dev/null && \
       echo "$metrics" | jq -e '.avg_tokens_per_second' > /dev/null && \
       echo "$metrics" | jq -e '.p50_first_token_ms' > /dev/null; then
        return 0
    fi
    return 1
}

test_models_endpoint() {
    # Check models list
    models=$(curl -s $MODELS_URL)
    
    if echo "$models" | jq -e '.models[0].id' > /dev/null; then
        return 0
    fi
    return 1
}

test_version_endpoint() {
    # Check version info
    version=$(curl -s $VERSION_URL | jq -r '.version')
    
    if [[ -n "$version" ]]; then
        return 0
    fi
    return 1
}

# Concurrent request test
test_concurrent_requests() {
    # Launch 3 concurrent requests
    (curl -s -X POST $API_URL -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Count to 3"}], "stream": true}' > /tmp/req1.txt) &
    (curl -s -X POST $API_URL -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "Say hello"}], "stream": true}' > /tmp/req2.txt) &
    (curl -s -X POST $API_URL -H "Content-Type: application/json" \
        -d '{"messages": [{"role": "user", "content": "What is 1+1?"}], "stream": true}' > /tmp/req3.txt) &
    
    wait
    
    # Check all completed
    if [ -s /tmp/req1.txt ] && [ -s /tmp/req2.txt ] && [ -s /tmp/req3.txt ]; then
        rm -f /tmp/req1.txt /tmp/req2.txt /tmp/req3.txt
        return 0
    fi
    return 1
}

# Main test execution
echo "============================================"
echo "ChatSafe Comprehensive Test Suite"
echo "============================================"
echo

# Check if server is running
if ! curl -s $HEALTH_URL > /dev/null 2>&1; then
    echo -e "${RED}Error: ChatSafe server is not running on port 8081${NC}"
    echo "Start the server with: ./target/release/chatsafe-server"
    exit 1
fi

echo "Running tests..."
echo

# Non-streaming golden tests
echo -e "${YELLOW}Golden Tests - Non-Streaming:${NC}"
run_test "Greeting Response" test_greeting_non_stream
run_test "Math Response" test_math_non_stream
run_test "Summarization" test_summarize_non_stream
run_test "Story Generation" test_story_non_stream
echo

# Streaming golden tests
echo -e "${YELLOW}Golden Tests - Streaming:${NC}"
run_test "Greeting Stream" test_greeting_stream
run_test "SSE Sequence" test_stream_sequence
echo

# Contract tests
echo -e "${YELLOW}Contract Tests:${NC}"
run_test "Parameter Bounds" test_parameter_bounds
run_test "Empty Messages" test_empty_messages
run_test "Max Tokens Limit" test_max_tokens
run_test "Role Pollution Check" test_no_role_pollution
echo

# Endpoint tests
echo -e "${YELLOW}Endpoint Tests:${NC}"
run_test "Health Endpoint" test_health_endpoint
run_test "Metrics Endpoint" test_metrics_endpoint
run_test "Models Endpoint" test_models_endpoint
run_test "Version Endpoint" test_version_endpoint
echo

# Concurrent tests
echo -e "${YELLOW}Concurrency Tests:${NC}"
run_test "Concurrent Requests" test_concurrent_requests
echo

# Summary
echo "============================================"
echo "Test Results:"
echo -e "  Total:  $TOTAL_TESTS"
echo -e "  Passed: ${GREEN}$PASSED_TESTS${NC}"
echo -e "  Failed: ${RED}$FAILED_TESTS${NC}"

if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "\n${GREEN}✓ All tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}✗ Some tests failed${NC}"
    exit 1
fi