#!/bin/bash

echo "ChatSafe - Local AI Assistant (Llama-3.2-3B)"
echo "Type 'exit' to quit"
echo "----------------------------------------"

# Use a temporary file to handle multi-line input safely
TMPFILE=$(mktemp)
trap "rm -f $TMPFILE" EXIT

while true; do
    echo -n "You: "
    
    # Read single line first
    IFS= read -r first_line
    
    if [ "$first_line" = "exit" ]; then
        echo "Goodbye!"
        break
    fi
    
    # Skip empty input
    if [ -z "$first_line" ]; then
        continue
    fi
    
    # Clear the temp file and add first line
    echo "$first_line" > "$TMPFILE"
    
    # Check if clipboard/paste buffer might have more lines
    # by checking if input looks incomplete or contains role markers
    if [[ "$first_line" =~ (AI:|You:|🎯|^Right now) ]]; then
        echo "(Detected possible multi-line paste. Press Ctrl+D when done)"
        echo -n "> "
        # Read remaining lines until EOF (Ctrl+D)
        cat >> "$TMPFILE"
    fi
    
    echo -n "AI: "
    
    # Read the complete input from temp file
    user_input=$(cat "$TMPFILE")
    
    # Create JSON payload using the file content
    json_payload=$(cat "$TMPFILE" | jq -Rs '{"messages": [{"role": "user", "content": .}], "stream": false}')
    
    # Send request
    response=$(curl -s -X POST http://127.0.0.1:8081/v1/chat/completions \
        -H "Content-Type: application/json" \
        -d "$json_payload" 2>/dev/null)
    
    # Check if curl succeeded
    if [ $? -ne 0 ]; then
        echo "Error: Failed to connect to ChatSafe server"
        echo
        continue
    fi
    
    # Extract and display the response
    content=$(echo "$response" | jq -r '.choices[0].message.content' 2>/dev/null)
    
    if [ "$content" = "null" ] || [ -z "$content" ]; then
        error=$(echo "$response" | jq -r '.error.message' 2>/dev/null)
        if [ "$error" != "null" ] && [ -n "$error" ]; then
            echo "Error: $error"
        else
            echo "Error: Invalid response from server"
        fi
    else
        echo "$content"
    fi
    
    echo
done