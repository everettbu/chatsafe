#!/bin/bash

# DoD Golden Behavior Tests
# Tests streaming SSE, proper role frames, and clean responses

BASE_URL="http://127.0.0.1:8081"
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo "🧪 Testing DoD Contract Requirements..."
echo

# Test 1: Greeting with streaming
echo "Test 1: Greeting (streaming)..."
response=$(curl -s -X POST "$BASE_URL/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "hi"}],
    "stream": true
  }' 2>&1)

# Check for proper SSE structure
if echo "$response" | grep -q "data:.*role.*assistant"; then
  echo -e "${GREEN}✅ Role frame present${NC}"
else
  echo -e "${RED}❌ Missing role frame${NC}"
fi

if echo "$response" | grep -q "\[DONE\]"; then
  echo -e "${GREEN}✅ [DONE] marker present${NC}"
else
  echo -e "${RED}❌ Missing [DONE] marker${NC}"
fi

# Check no role pollution
if echo "$response" | grep -qE "(AI:|Assistant:|User:)"; then
  echo -e "${RED}❌ Role pollution detected${NC}"
else
  echo -e "${GREEN}✅ No role pollution${NC}"
fi
echo

# Test 2: Math (non-streaming)
echo "Test 2: Math (non-streaming)..."
response=$(curl -s -X POST "$BASE_URL/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "What is 2+2?"}],
    "stream": false
  }')

content=$(echo "$response" | jq -r '.choices[0].message.content' 2>/dev/null)
if echo "$content" | grep -q "4"; then
  echo -e "${GREEN}✅ Math correct: $content${NC}"
else
  echo -e "${RED}❌ Math incorrect: $content${NC}"
fi

# Check for disclaimers
if echo "$content" | grep -qiE "(as an ai|I cannot|I don.t have access)"; then
  echo -e "${RED}❌ Unnecessary disclaimers detected${NC}"
else
  echo -e "${GREEN}✅ No disclaimers${NC}"
fi
echo

# Test 3: /models endpoint
echo "Test 3: /models endpoint..."
response=$(curl -s "$BASE_URL/v1/models")
if echo "$response" | jq -r '.data[0].id' | grep -q "llama-3.2-3b-instruct"; then
  echo -e "${GREEN}✅ /models endpoint working${NC}"
else
  echo -e "${RED}❌ /models endpoint issue${NC}"
fi
echo

# Test 4: /version endpoint  
echo "Test 4: /version endpoint..."
response=$(curl -s "$BASE_URL/version")
if echo "$response" | jq -r '.model_id' | grep -q "llama-3.2-3b-instruct"; then
  echo -e "${GREEN}✅ /version endpoint working${NC}"
else
  echo -e "${RED}❌ /version endpoint issue${NC}"
fi
echo

# Test 5: Streaming chunks (check for multiple data events)
echo "Test 5: Multiple streaming chunks..."
response=$(curl -s -X POST "$BASE_URL/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "Tell me a short story about a mountain."}],
    "stream": true,
    "max_tokens": 100
  }' 2>&1)

chunk_count=$(echo "$response" | grep -c "^data:")
if [ "$chunk_count" -gt 3 ]; then
  echo -e "${GREEN}✅ Multiple chunks detected: $chunk_count chunks${NC}"
else
  echo -e "${RED}❌ Insufficient chunks: $chunk_count chunks (expected >3)${NC}"
fi

echo
echo "🎯 DoD Test Summary Complete"