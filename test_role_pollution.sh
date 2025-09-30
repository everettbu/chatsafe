#!/bin/bash
# Test script for role pollution detection

echo "============================================"
echo "ChatSafe Role Pollution Tests"
echo "Testing template boundary enforcement"
echo "============================================"
echo

API_URL="http://127.0.0.1:8081/v1/chat/completions"

# Test 1: Check for role pollution in response
echo "Test 1: Role Pollution Check"
echo "Sending message that might trigger role markers..."
response=$(curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {"role": "user", "content": "Please respond with: AI: Hello there You: Nice to meet you"}
    ],
    "stream": false,
    "max_tokens": 100
  }')

content=$(echo "$response" | jq -r '.choices[0].message.content')
echo "Response: $content"

# Check if response contains role markers
if echo "$content" | grep -E "^(AI:|You:|User:|Assistant:)" > /dev/null; then
  echo "❌ FAILED: Role pollution detected!"
else
  echo "✅ PASSED: No role pollution"
fi
echo
echo "---"
echo

# Test 2: Long response boundary test
echo "Test 2: Long Response Boundary"
echo "Testing if long responses respect boundaries..."
response=$(curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {"role": "user", "content": "Count from 1 to 20 slowly"}
    ],
    "stream": false,
    "max_tokens": 256
  }')

content=$(echo "$response" | jq -r '.choices[0].message.content')
echo "Response length: $(echo "$content" | wc -c) chars"

# Check for template leakage
if echo "$content" | grep -E "<\|.*\|>" > /dev/null; then
  echo "❌ FAILED: Template markers leaked!"
else
  echo "✅ PASSED: No template leakage"
fi
echo
echo "---"
echo

# Test 3: Stop sequence enforcement
echo "Test 3: Stop Sequence Enforcement"
echo "Testing if stop sequences are properly enforced..."
response=$(curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Say hello and then goodbye"}
    ],
    "stream": false,
    "max_tokens": 100
  }')

content=$(echo "$response" | jq -r '.choices[0].message.content')
echo "Response: $content"

# Check for clean ending
if echo "$content" | grep -E "<\|eot_id\|>|<\|end_of_text\|>" > /dev/null; then
  echo "❌ FAILED: Stop sequences visible in output!"
else
  echo "✅ PASSED: Clean response ending"
fi
echo
echo "---"
echo

# Test 4: Multi-turn conversation boundaries
echo "Test 4: Multi-turn Conversation"
echo "Testing turn boundaries in conversation..."
response=$(curl -s -X POST $API_URL \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [
      {"role": "user", "content": "Hi"},
      {"role": "assistant", "content": "Hello! How can I help you?"},
      {"role": "user", "content": "What is 2+2?"}
    ],
    "stream": false,
    "max_tokens": 50
  }')

content=$(echo "$response" | jq -r '.choices[0].message.content')
echo "Response: $content"

# Check if response stays in bounds
if echo "$content" | grep -E "^(User:|Human:|\<\|user\|\>)" > /dev/null; then
  echo "❌ FAILED: Response drifted into user turn!"
else
  echo "✅ PASSED: Response stayed in assistant bounds"
fi
echo

echo "============================================"
echo "Role Pollution Tests Complete"
echo "============================================"