#!/usr/bin/env python3
"""
ChatSafe Interactive Chat Client
Handles multi-line input properly and escapes JSON correctly
"""

import json
import requests
import sys
import readline  # For better input handling

def send_message(message):
    """Send a message to the ChatSafe API and return the response"""
    url = "http://127.0.0.1:8081/v1/chat/completions"
    
    # Clean up any role prefixes that might confuse the model
    cleaned_message = message.replace("AI: ", "").replace("You: ", "")
    
    payload = {
        "messages": [{"role": "user", "content": cleaned_message}],
        "stream": False
    }
    
    try:
        response = requests.post(url, json=payload)
        response.raise_for_status()
        
        data = response.json()
        return data.get("choices", [{}])[0].get("message", {}).get("content", "No response")
    
    except requests.ConnectionError:
        return "Error: Failed to connect to ChatSafe server. Is it running?"
    except requests.HTTPError as e:
        return f"Error: HTTP {e.response.status_code}"
    except json.JSONDecodeError:
        return "Error: Invalid response from server"
    except Exception as e:
        return f"Error: {e}"

def main():
    print("ChatSafe - Local AI Assistant (Llama-3.2-3B)")
    print("Type 'exit' to quit, 'clear' to clear screen")
    print("Paste multi-line input and press Enter twice to send")
    print("----------------------------------------")
    
    while True:
        try:
            # Collect input (handles multi-line naturally)
            lines = []
            print("You: ", end="", flush=True)
            
            while True:
                line = input()
                
                # Handle single-line commands
                if not lines and line == "exit":
                    print("Goodbye!")
                    return
                
                if not lines and line == "clear":
                    print("\033[2J\033[H")  # Clear screen
                    print("ChatSafe - Local AI Assistant (Llama-3.2-3B)")
                    print("Type 'exit' to quit, 'clear' to clear screen")
                    print("Paste multi-line input and press Enter twice to send")
                    print("----------------------------------------")
                    break
                
                # Empty line after content = send message
                if not line and lines:
                    break
                    
                # Empty line with no content = skip
                if not line and not lines:
                    break
                    
                lines.append(line)
                print("> ", end="", flush=True)  # Continuation prompt
            
            # If we have content, send it
            if lines:
                message = "\n".join(lines)
                print("AI: ", end="", flush=True)
                response = send_message(message)
                print(response)
                print()
        
        except KeyboardInterrupt:
            print("\n\nGoodbye!")
            break
        except EOFError:
            print("\nGoodbye!")
            break

if __name__ == "__main__":
    main()