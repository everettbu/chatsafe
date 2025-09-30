#[cfg(test)]
mod pollution_tests {
    use crate::template_engine::{TemplateEngine, CleanedResponse, StreamChunkResult};
    use chatsafe_common::{Message, Role};
    use chatsafe_config::TemplateConfig;
    
    fn llama3_template() -> TemplateConfig {
        TemplateConfig {
            id: "llama3".to_string(),
            name: "Llama 3".to_string(),
            system_prefix: "<|start_header_id|>system<|end_header_id|>\n\n".to_string(),
            system_suffix: "<|eot_id|>".to_string(),
            user_prefix: "<|start_header_id|>user<|end_header_id|>\n\n".to_string(),
            user_suffix: "<|eot_id|>".to_string(),
            assistant_prefix: "<|start_header_id|>assistant<|end_header_id|>\n\n".to_string(),
            assistant_suffix: "<|eot_id|>".to_string(),
            default_system_prompt: "You are helpful.".to_string(),
        }
    }
    
    #[test]
    fn test_no_role_pollution_in_clean_response() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        
        // Response with role pollution
        let polluted = "AI: Hello there!\nYou: How are you?\nI'm doing well, thank you.";
        let cleaned = TemplateEngine::clean_response(
            polluted,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        // Should remove role markers
        assert!(!cleaned.content.contains("AI:"));
        assert!(!cleaned.content.contains("You:"));
        assert!(cleaned.content.contains("Hello there!"));
        assert!(cleaned.content.contains("How are you?"));
    }
    
    #[test]
    fn test_stop_at_turn_boundary() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        
        // Response that tries to continue into next turn
        let response = "Here is my answer.<|eot_id|><|start_header_id|>user<|end_header_id|>Thanks!";
        let cleaned = TemplateEngine::clean_response(
            response,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        // Should stop at turn boundary
        assert_eq!(cleaned.content, "Here is my answer.");
        assert_eq!(cleaned.stopped_at, Some("<|eot_id|>".to_string()));
    }
    
    #[test]
    fn test_remove_template_markers() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        
        // Response with leaked template markers
        let response = "<|start_header_id|>assistant<|end_header_id|>\n\nHello there!";
        let cleaned = TemplateEngine::clean_response(
            response,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        // Should remove all template markers
        assert!(!cleaned.content.contains("<|start_header_id|>"));
        assert!(!cleaned.content.contains("<|end_header_id|>"));
        assert!(!cleaned.content.contains("assistant"));
        assert_eq!(cleaned.content, "Hello there!");
    }
    
    #[test]
    fn test_long_response_boundary() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        
        // Long response that might drift
        let mut long_response = String::new();
        for i in 1..=100 {
            long_response.push_str(&format!("Line {}. ", i));
        }
        long_response.push_str("<|eot_id|>Extra content after boundary");
        
        let cleaned = TemplateEngine::clean_response(
            &long_response,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        // Should stop at boundary
        assert!(!cleaned.content.contains("Extra content"));
        assert!(cleaned.content.contains("Line 100"));
        assert_eq!(cleaned.stopped_at, Some("<|eot_id|>".to_string()));
    }
    
    #[test]
    fn test_streaming_role_pollution_removal() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        let mut buffer = String::new();
        
        // Stream chunk with role pollution
        let chunk = "AI: This is a response\nContinuing...";
        let result = TemplateEngine::process_stream_chunk(
            chunk,
            &template,
            &stop_sequences,
            eos_token,
            &mut buffer,
        );
        
        match result {
            StreamChunkResult::Partial { content } => {
                assert!(!content.contains("AI:"));
                assert!(content.contains("This is a response"));
            }
            _ => panic!("Expected partial result"),
        }
    }
    
    #[test]
    fn test_streaming_stop_detection() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        let mut buffer = String::new();
        
        // Stream chunks that build up to stop sequence
        let chunk1 = "Hello world";
        let result1 = TemplateEngine::process_stream_chunk(
            chunk1,
            &template,
            &stop_sequences,
            eos_token,
            &mut buffer,
        );
        
        assert!(matches!(result1, StreamChunkResult::Partial { .. }));
        
        let chunk2 = "<|eot_id|>Extra";
        let result2 = TemplateEngine::process_stream_chunk(
            chunk2,
            &template,
            &stop_sequences,
            eos_token,
            &mut buffer,
        );
        
        match result2 {
            StreamChunkResult::Complete { content, stopped_at } => {
                assert_eq!(content, "Hello world");
                assert_eq!(stopped_at, Some("<|eot_id|>".to_string()));
            }
            _ => panic!("Expected complete result"),
        }
    }
    
    #[test]
    fn test_multi_turn_prompt_formatting() {
        let template = llama3_template();
        let messages = vec![
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
        
        // Check proper formatting
        assert!(prompt.contains("<|start_header_id|>user<|end_header_id|>\n\nHello<|eot_id|>"));
        assert!(prompt.contains("<|start_header_id|>assistant<|end_header_id|>\n\nHi there!<|eot_id|>"));
        assert!(prompt.contains("<|start_header_id|>user<|end_header_id|>\n\nHow are you?<|eot_id|>"));
        assert!(prompt.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    }
    
    #[test]
    fn test_defensive_cleaning() {
        let template = llama3_template();
        let stop_sequences = vec!["<|eot_id|>".to_string()];
        let eos_token = "<|end_of_text|>";
        
        // Malformed response with multiple issues
        let malformed = "AI: <|start_header_id|>Hello\nYou: there<|eot_id|>\nUser: How<|end_of_text|> are you?";
        let cleaned = TemplateEngine::clean_response(
            malformed,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        // Should clean all issues
        assert!(!cleaned.content.contains("AI:"));
        assert!(!cleaned.content.contains("You:"));
        assert!(!cleaned.content.contains("User:"));
        assert!(!cleaned.content.contains("<|start_header_id|>"));
        assert!(!cleaned.content.contains("<|eot_id|>"));
        assert!(cleaned.content.contains("Hello"));
        assert!(cleaned.content.contains("there"));
    }
}