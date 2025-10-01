#[cfg(test)]
mod tests {
    use super::*;
    use crate::template_engine::{TemplateEngine, StreamChunkResult};
    use chatsafe_common::{Message, Role};
    use chatsafe_config::TemplateConfig;

    fn test_template() -> TemplateConfig {
        TemplateConfig {
            id: "test".to_string(),
            name: "Test Template".to_string(),
            system_prefix: "<|start_header_id|>system<|end_header_id|>\n\n".to_string(),
            system_suffix: "<|eot_id|>".to_string(),
            user_prefix: "<|start_header_id|>user<|end_header_id|>\n\n".to_string(),
            user_suffix: "<|eot_id|>".to_string(),
            assistant_prefix: "<|start_header_id|>assistant<|end_header_id|>\n\n".to_string(),
            assistant_suffix: "<|eot_id|>".to_string(),
            default_system_prompt: "You are a helpful assistant.".to_string(),
        }
    }

    #[test]
    fn test_no_role_pollution_in_response() {
        let template = test_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        
        // Test various role pollution patterns
        let polluted_responses = vec![
            "AI: This is my response",
            "You: asked a question\nAI: Here's my answer",
            "User: What?\nAssistant: Let me help",
            "\n\nHuman: test\nBot: response",
        ];
        
        for response in polluted_responses {
            let cleaned = TemplateEngine::clean_response(
                response,
                &template,
                &stop_sequences,
                eos_token,
            );
            
            // Should not contain role markers at line start
            assert!(!cleaned.content.lines().any(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("AI:") || 
                trimmed.starts_with("You:") || 
                trimmed.starts_with("User:") ||
                trimmed.starts_with("Assistant:") ||
                trimmed.starts_with("Human:") ||
                trimmed.starts_with("Bot:")
            }), "Role pollution not cleaned: {}", cleaned.content);
        }
    }

    #[test]
    fn test_stop_sequence_detection() {
        let template = test_template();
        let stop_sequences = vec![
            "<|eot_id|>".to_string(),
            "<|end_of_text|>".to_string(),
        ];
        let eos_token = "<|endoftext|>";
        
        let test_cases = vec![
            ("Hello world<|eot_id|>extra", "Hello world", Some("<|eot_id|>".to_string())),
            ("Response<|end_of_text|>", "Response", Some("<|end_of_text|>".to_string())),
            ("Complete<|endoftext|>ignored", "Complete", Some("<|endoftext|>".to_string())),
            ("No stop sequence here", "No stop sequence here", None),
        ];
        
        for (input, expected_content, expected_stop) in test_cases {
            let cleaned = TemplateEngine::clean_response(
                input,
                &template,
                &stop_sequences,
                eos_token,
            );
            
            assert_eq!(cleaned.content, expected_content);
            assert_eq!(cleaned.stopped_at, expected_stop);
        }
    }

    #[test]
    fn test_streaming_chunk_processing() {
        let template = test_template();
        let stop_sequences = vec!["STOP".to_string()];
        let eos_token = "END";
        let mut buffer = String::new();
        
        // Simulate streaming tokens
        let chunks = vec![
            "Hello ",
            "world",
            "! This ",
            "is streaming",
            " STOP",
            " ignored",
        ];
        
        let mut accumulated = String::new();
        let mut stopped = false;
        
        for chunk in chunks {
            if stopped {
                break;
            }
            
            let result = TemplateEngine::process_stream_chunk(
                chunk,
                &template,
                &stop_sequences,
                eos_token,
                &mut buffer,
            );
            
            match result {
                StreamChunkResult::Partial { content } => {
                    accumulated.push_str(&content);
                }
                StreamChunkResult::Complete { content, stopped_at } => {
                    // Complete returns the full cleaned content, not a delta
                    accumulated = content;
                    assert_eq!(stopped_at, Some("STOP".to_string()));
                    stopped = true;
                }
                StreamChunkResult::Buffering => {
                    // Continue buffering
                }
            }
        }
        
        // The streaming should stop before STOP and trim
        assert_eq!(accumulated.trim(), "Hello world! This is streaming");
    }

    #[test]
    fn test_prompt_formatting() {
        let template = test_template();
        
        let messages = vec![
            Message {
                role: Role::System,
                content: "Be concise.".to_string(),
            },
            Message {
                role: Role::User,
                content: "Hello".to_string(),
            },
            Message {
                role: Role::Assistant,
                content: "Hi there!".to_string(),
            },
            Message {
                role: Role::User,
                content: "How are you?".to_string(),
            },
        ];
        
        let prompt = TemplateEngine::format_prompt(&messages, &template);
        
        // Check structure
        assert!(prompt.contains("<|start_header_id|>system<|end_header_id|>"));
        assert!(prompt.contains("Be concise."));
        assert!(prompt.contains("<|start_header_id|>user<|end_header_id|>"));
        assert!(prompt.contains("Hello"));
        assert!(prompt.contains("<|start_header_id|>assistant<|end_header_id|>"));
        assert!(prompt.contains("Hi there!"));
        assert!(prompt.contains("How are you?"));
        
        // Should end with assistant prefix for generation
        assert!(prompt.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    }

    #[test]
    fn test_clean_multi_line_response() {
        let template = test_template();
        let stop_sequences = vec![];
        let eos_token = "END";
        
        let response = "Line 1\nAI: Line 2\nLine 3\nYou: Line 4";
        let cleaned = TemplateEngine::clean_response(
            response,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        // When both AI: and You: are present, returns replacement message
        assert!(cleaned.content.contains("I understand you'd like me to respond"));
        assert!(!cleaned.content.contains("AI:"));
        assert!(!cleaned.content.contains("You:"));
    }
}