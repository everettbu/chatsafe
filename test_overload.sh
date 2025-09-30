#!/bin/bash

# Test for role pollution and instruction ingestion issues

echo "Testing overload scenario that causes role pollution..."
echo

# Create a complex prompt that might trigger the issue
COMPLEX_PROMPT='Please explain the following in detail: What are the key differences between synchronous and asynchronous programming? Include examples in JavaScript. Also explain event loops, callbacks, promises, and async/await. Make sure to cover error handling patterns. Then provide a comparison with other languages like Python and Rust. Finally, discuss best practices for debugging async code.'

echo "Sending complex prompt..."
response=$(curl -s -X POST http://127.0.0.1:8081/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d "{
        \"messages\": [
            {\"role\": \"user\", \"content\": \"$COMPLEX_PROMPT\"}
        ],
        \"stream\": false,
        \"max_tokens\": 512
    }")

echo "Response:"
echo "$response" | jq -r '.choices[0].message.content' 2>/dev/null || echo "$response"
echo
echo "---"
echo

# Check for role pollution
content=$(echo "$response" | jq -r '.choices[0].message.content' 2>/dev/null)
if echo "$content" | grep -E "(AI:|Assistant:|You:|User:)" > /dev/null 2>&1; then
    echo "⚠️  ROLE POLLUTION DETECTED!"
    echo "$content" | grep -E "(AI:|Assistant:|You:|User:)" | head -5
else
    echo "✅ No role pollution detected"
fi

# Check for template leakage
if echo "$content" | grep -E "(<\|.*\|>|\[INST\]|\[/INST\])" > /dev/null 2>&1; then
    echo "⚠️  TEMPLATE LEAKAGE DETECTED!"
    echo "$content" | grep -E "(<\|.*\|>|\[INST\]|\[/INST\])" | head -5
else
    echo "✅ No template leakage detected"
fi