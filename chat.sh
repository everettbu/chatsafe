#!/bin/bash

echo "ChatSafe - Local AI Assistant (Llama-3.2-3B)"
echo "Type 'exit' to quit"
echo "----------------------------------------"

while true; do
    echo -n "You: "
    read user_input
    
    if [ "$user_input" = "exit" ]; then
        echo "Goodbye!"
        break
    fi
    
    echo -n "AI: "
    response=$(curl -s -X POST http://127.0.0.1:8081/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d "{\"messages\": [{\"role\": \"user\", \"content\": \"$user_input\"}], \"stream\": false}")
    
    echo "$response" | jq -r '.choices[0].message.content' 2>/dev/null || echo "$response"
    echo
done