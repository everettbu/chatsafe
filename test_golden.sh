#!/bin/bash
# Golden behavior test script for Llama-3.2-3B-Instruct

echo "============================================"
echo "ChatSafe Golden Behavior Tests"
echo "Model: Llama-3.2-3B-Instruct Q4_K_M"
echo "============================================"
echo

API_URL="http://127.0.0.1:8081/v1/chat/completions"

# Test 1: Simple greeting
echo "Test 1: Greeting (expect ≤2 sentences)"
echo "Input: 'Hello!'"
curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "Hello!"}], "stream": false}' | \
  jq -r '.choices[0].message.content'
echo
echo "---"
echo

# Test 2: Math
echo "Test 2: Math (expect '4' or '2 + 2 = 4')"
echo "Input: 'What is 2+2?'"
curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "What is 2+2?"}], "stream": false}' | \
  jq -r '.choices[0].message.content'
echo
echo "---"
echo

# Test 3: Summarization
echo "Test 3: Summarization (expect 2-5 lines)"
echo "Input: 'Summarize: The quick brown fox jumps over the lazy dog. This pangram contains all letters of the alphabet and is commonly used for testing.'"
curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "Summarize: The quick brown fox jumps over the lazy dog. This pangram contains all letters of the alphabet and is commonly used for testing."}], "stream": false}' | \
  jq -r '.choices[0].message.content'
echo
echo "---"
echo

# Test 4: Story
echo "Test 4: Story (expect 3-5 coherent sentences)"
echo "Input: 'Tell a 3-sentence story about a cat.'"
curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "Tell a 3-sentence story about a cat."}], "stream": false}' | \
  jq -r '.choices[0].message.content'
echo
echo "---"
echo

# Test 5: Check streaming
echo "Test 5: Streaming response"
echo "Input: 'Count to 3'"
curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{"messages": [{"role": "user", "content": "Count to 3"}], "stream": true}' | \
  head -5
echo
echo "============================================"
echo "Tests Complete"
echo "============================================"