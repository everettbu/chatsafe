use chatsafe_common::{Message, Role};
use chatsafe_config::TemplateConfig;
use std::collections::HashSet;

/// Template engine for formatting messages and cleaning responses
pub struct TemplateEngine;

impl TemplateEngine {
    /// Format messages into a prompt using the model template
    pub fn format_prompt(
        messages: &[Message],
        template: &TemplateConfig,
    ) -> String {
        let mut prompt = String::new();
        let mut has_system = false;
        
        for message in messages {
            match message.role {
                Role::System => {
                    has_system = true;
                    prompt.push_str(&template.system_prefix);
                    prompt.push_str(&message.content);
                    prompt.push_str(&template.system_suffix);
                }
                Role::User => {
                    // Add default system prompt if not provided
                    if !has_system {
                        has_system = true;
                        prompt.push_str(&template.system_prefix);
                        prompt.push_str(&template.default_system_prompt);
                        prompt.push_str(&template.system_suffix);
                    }
                    
                    prompt.push_str(&template.user_prefix);
                    prompt.push_str(&message.content);
                    prompt.push_str(&template.user_suffix);
                }
                Role::Assistant => {
                    prompt.push_str(&template.assistant_prefix);
                    prompt.push_str(&message.content);
                    prompt.push_str(&template.assistant_suffix);
                }
            }
        }
        
        // Add assistant prefix to signal model to respond
        prompt.push_str(&template.assistant_prefix);
        
        prompt
    }
    
    /// Clean response by removing template markers and role pollution
    pub fn clean_response(
        response: &str,
        template: &TemplateConfig,
        stop_sequences: &[String],
        eos_token: &str,
    ) -> CleanedResponse {
        let mut cleaned = response.to_string();
        let mut stopped_at: Option<String> = None;
        
        // First, detect and truncate at stop sequences
        for stop_seq in stop_sequences.iter().chain(std::iter::once(&eos_token.to_string())) {
            if let Some(pos) = cleaned.find(stop_seq.as_str()) {
                cleaned.truncate(pos);
                stopped_at = Some(stop_seq.clone());
                break;
            }
        }
        
        // Remove any template markers that shouldn't be in the output
        let markers_to_remove = vec![
            &template.assistant_suffix,
            &template.user_prefix,
            &template.system_prefix,
            &template.assistant_prefix, // Remove if model echoes its own prefix
            "<|eot_id|>",
            "<|end_of_text|>",
            "<|start_header_id|>",
            "<|im_end|>",
            "<|im_start|>",
        ];
        
        for marker in markers_to_remove {
            if !marker.is_empty() {
                cleaned = cleaned.replace(marker, "");
            }
        }
        
        // Remove role pollution (lines starting with role markers)
        cleaned = Self::remove_role_pollution(&cleaned);
        
        // Final trim
        cleaned = cleaned.trim().to_string();
        
        CleanedResponse {
            content: cleaned,
            stopped_at,
        }
    }
    
    /// Remove role pollution from response
    fn remove_role_pollution(text: &str) -> String {
        // If the entire response looks like role-play output, reject it
        if text.contains("AI:") && text.contains("You:") {
            // This appears to be the model outputting a dialogue
            // Return a generic response instead
            return "I understand you'd like me to respond, but I should avoid role-playing conversations. How can I help you directly?".to_string();
        }
        
        let role_patterns = vec![
            "AI:", "You:", "User:", "Assistant:", "System:",
            "Human:", "Bot:", "### Instruction:", "### Response:",
        ];
        
        let lines: Vec<&str> = text.lines().collect();
        let mut cleaned_lines = Vec::new();
        
        for line in lines {
            let mut clean_line = line.to_string();
            
            // Check if line starts with any role pattern
            for pattern in &role_patterns {
                if line.trim_start().starts_with(pattern) {
                    // If this is the ONLY content on the line after the role marker,
                    // it might be the model trying to output a dialogue
                    let remainder = line.trim_start()
                        .trim_start_matches(pattern)
                        .trim();
                    
                    // If there's actual content after the role marker, keep it
                    if !remainder.is_empty() {
                        clean_line = remainder.to_string();
                    } else {
                        // Empty after role marker, skip this line
                        continue;
                    }
                    break;
                }
            }
            
            // Also check for role markers mid-line (less aggressive)
            for pattern in &role_patterns {
                if clean_line.contains(pattern) && !clean_line.starts_with(pattern) {
                    // Only remove if it looks like an obvious role marker
                    // (e.g., at the start of a sentence)
                    clean_line = clean_line.replace(&format!("\n{}", pattern), "\n");
                    clean_line = clean_line.replace(&format!(". {}", pattern), ". ");
                }
            }
            
            if !clean_line.is_empty() {
                cleaned_lines.push(clean_line);
            }
        }
        
        let result = cleaned_lines.join("\n").trim().to_string();
        
        // If we've removed everything, return a safe response
        if result.is_empty() {
            return "I'm here to help. What would you like to know?".to_string();
        }
        
        result
    }
    
    /// Check if text contains stop sequence
    pub fn contains_stop_sequence(
        text: &str,
        stop_sequences: &[String],
        eos_token: &str,
    ) -> Option<String> {
        for stop_seq in stop_sequences.iter().chain(std::iter::once(&eos_token.to_string())) {
            if text.contains(stop_seq) {
                return Some(stop_seq.clone());
            }
        }
        None
    }
    
    /// Process streaming chunk
    pub fn process_stream_chunk(
        chunk: &str,
        template: &TemplateConfig,
        stop_sequences: &[String],
        eos_token: &str,
        buffer: &mut String,
    ) -> StreamChunkResult {
        buffer.push_str(chunk);
        
        // Check for stop sequences in buffer
        if let Some(stop_seq) = Self::contains_stop_sequence(buffer, stop_sequences, eos_token) {
            // Found stop sequence - clean and finalize
            let cleaned = Self::clean_response(buffer, template, stop_sequences, eos_token);
            buffer.clear();
            
            StreamChunkResult::Complete {
                content: cleaned.content,
                stopped_at: Some(stop_seq),
            }
        } else {
            // Check if we can emit a partial chunk safely
            // We want to avoid emitting partial template markers
            let safe_emit_pos = Self::find_safe_emit_position(buffer, template);
            
            if safe_emit_pos > 0 {
                let to_emit = buffer[..safe_emit_pos].to_string();
                *buffer = buffer[safe_emit_pos..].to_string();
                
                // Clean the chunk before emitting
                let cleaned = Self::remove_role_pollution(&to_emit);
                
                StreamChunkResult::Partial {
                    content: cleaned,
                }
            } else {
                // Not enough data to emit safely yet
                StreamChunkResult::Buffering
            }
        }
    }
    
    /// Find a safe position to emit content without breaking template markers
    fn find_safe_emit_position(buffer: &str, template: &TemplateConfig) -> usize {
        // Collect all template markers to watch for
        let markers: HashSet<&str> = vec![
            &template.system_prefix,
            &template.system_suffix,
            &template.user_prefix,
            &template.user_suffix,
            &template.assistant_prefix,
            &template.assistant_suffix,
            "<|",  // Common marker prefix
            "###", // Common instruction marker
        ].into_iter()
        .filter(|s| !s.is_empty())
        .collect();
        
        // Find the last safe position to emit
        let mut safe_pos = 0;
        let chars: Vec<char> = buffer.chars().collect();
        
        for i in 0..chars.len() {
            let remaining = buffer[i..].to_string();
            
            // Check if we're at the start of any marker
            let mut at_marker_start = false;
            for marker in &markers {
                if remaining.starts_with(marker) || marker.starts_with(&remaining) {
                    at_marker_start = true;
                    break;
                }
            }
            
            if !at_marker_start {
                safe_pos = i + chars[i].len_utf8();
            } else {
                break;
            }
        }
        
        safe_pos
    }
}

/// Result of cleaning a response
#[derive(Debug, Clone)]
pub struct CleanedResponse {
    pub content: String,
    pub stopped_at: Option<String>,
}

/// Result of processing a stream chunk
#[derive(Debug, Clone)]
pub enum StreamChunkResult {
    /// Partial content that can be emitted
    Partial { content: String },
    /// Complete response detected
    Complete { content: String, stopped_at: Option<String> },
    /// Still buffering, not ready to emit
    Buffering,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn test_template() -> TemplateConfig {
        TemplateConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            system_prefix: "<|system|>".to_string(),
            system_suffix: "</|system|>".to_string(),
            user_prefix: "<|user|>".to_string(),
            user_suffix: "</|user|>".to_string(),
            assistant_prefix: "<|assistant|>".to_string(),
            assistant_suffix: "</|assistant|>".to_string(),
            default_system_prompt: "You are helpful.".to_string(),
        }
    }
    
    #[test]
    fn test_format_prompt() {
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
        ];
        
        let prompt = TemplateEngine::format_prompt(&messages, &template);
        
        assert!(prompt.contains("<|system|>Be concise.</|system|>"));
        assert!(prompt.contains("<|user|>Hello</|user|>"));
        assert!(prompt.ends_with("<|assistant|>"));
    }
    
    #[test]
    fn test_clean_response() {
        let template = test_template();
        let stop_sequences = vec!["<|stop|>".to_string()];
        let eos_token = "<|eos|>";
        
        // Test stop sequence detection
        let response = "Hello world<|stop|>Extra content";
        let cleaned = TemplateEngine::clean_response(
            response,
            &template,
            &stop_sequences,
            eos_token,
        );
        
        assert_eq!(cleaned.content, "Hello world");
        assert_eq!(cleaned.stopped_at, Some("<|stop|>".to_string()));
    }
    
    #[test]
    fn test_remove_role_pollution() {
        // Test with both AI: and You: triggers replacement
        let text = "AI: This is a response\nNormal line\nYou: Should be removed\nAnother line";
        let cleaned = TemplateEngine::remove_role_pollution(text);
        
        // When both AI: and You: are present, it returns replacement message
        assert!(cleaned.contains("I understand you'd like me to respond"));
        assert!(!cleaned.contains("AI:"));
        assert!(!cleaned.contains("You:"));
        
        // Test with only one role marker - should clean but not replace
        let text_single = "AI: This is a response\nNormal line\nAnother line";
        let cleaned_single = TemplateEngine::remove_role_pollution(text_single);
        
        assert!(cleaned_single.contains("This is a response"));
        assert!(cleaned_single.contains("Normal line"));
        assert!(!cleaned_single.contains("AI:"));
    }
    
    #[test]
    fn test_stream_chunk_processing() {
        let template = test_template();
        let stop_sequences = vec!["STOP".to_string()];
        let eos_token = "EOS";
        let mut buffer = String::new();
        
        // Test partial chunk
        let result = TemplateEngine::process_stream_chunk(
            "Hello ",
            &template,
            &stop_sequences,
            eos_token,
            &mut buffer,
        );
        
        match result {
            StreamChunkResult::Partial { content } => {
                assert_eq!(content, "Hello ");
            }
            _ => panic!("Expected partial result"),
        }
        
        // Test stop sequence detection
        buffer.clear();
        let result = TemplateEngine::process_stream_chunk(
            "Hello STOP there",
            &template,
            &stop_sequences,
            eos_token,
            &mut buffer,
        );
        
        match result {
            StreamChunkResult::Complete { content, stopped_at } => {
                assert_eq!(content, "Hello");
                assert_eq!(stopped_at, Some("STOP".to_string()));
            }
            _ => panic!("Expected complete result"),
        }
    }
}