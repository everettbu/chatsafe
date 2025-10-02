use chatsafe_common::{Message, Role};
use chatsafe_config::TemplateConfig;
use std::fmt::Write;

// Constants for template markers
const TEMPLATE_MARKERS: &[&str] = &[
    "<|eot_id|>",
    "<|end_of_text|>",
    "<|start_header_id|>",
    "<|end_header_id|>",
    "<|im_end|>",
    "<|im_start|>",
];

// Constants for role patterns
const ROLE_PATTERNS: &[&str] = &[
    "AI:", "You:", "User:", "Assistant:", "System:",
    "Human:", "Bot:", "### Instruction:", "### Response:",
];

// Fallback messages
const ROLE_POLLUTION_FALLBACK: &str = "I understand you'd like me to respond, but I should avoid role-playing conversations. How can I help you directly?";
const EMPTY_RESPONSE_FALLBACK: &str = "I'm here to help. What would you like to know?";

/// Template engine for formatting messages and cleaning responses
pub struct TemplateEngine;

impl TemplateEngine {
    /// Format messages into a prompt using the model template
    pub fn format_prompt(
        messages: &[Message],
        template: &TemplateConfig,
    ) -> String {
        let mut prompt = String::with_capacity(1024); // Pre-allocate reasonable size
        let mut has_system = false;
        
        for message in messages {
            match message.role {
                Role::System => {
                    has_system = true;
                    Self::write_message(
                        &mut prompt,
                        &template.system_prefix,
                        &message.content,
                        &template.system_suffix,
                    );
                }
                Role::User => {
                    // Add default system prompt if not provided
                    if !has_system {
                        has_system = true;
                        Self::write_message(
                            &mut prompt,
                            &template.system_prefix,
                            &template.default_system_prompt,
                            &template.system_suffix,
                        );
                    }
                    
                    Self::write_message(
                        &mut prompt,
                        &template.user_prefix,
                        &message.content,
                        &template.user_suffix,
                    );
                }
                Role::Assistant => {
                    Self::write_message(
                        &mut prompt,
                        &template.assistant_prefix,
                        &message.content,
                        &template.assistant_suffix,
                    );
                }
            }
        }
        
        // Add assistant prefix to signal model to respond
        prompt.push_str(&template.assistant_prefix);
        
        prompt
    }
    
    /// Helper to write a message with prefix and suffix
    fn write_message(prompt: &mut String, prefix: &str, content: &str, suffix: &str) {
        // Pre-calculate capacity for better performance
        prompt.reserve(prefix.len() + content.len() + suffix.len());
        prompt.push_str(prefix);
        prompt.push_str(content);
        prompt.push_str(suffix);
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
        stopped_at = Self::truncate_at_stop_sequence(&mut cleaned, stop_sequences, eos_token);
        
        // Remove template prefixes/suffixes if they were echoed by the model
        Self::remove_template_echoes(&mut cleaned, template);
        
        // Remove any leaked template markers
        Self::remove_template_markers(&mut cleaned);
        
        // Remove role pollution
        cleaned = Self::remove_role_pollution(&cleaned);
        
        // Final trim
        cleaned = cleaned.trim().to_string();
        
        CleanedResponse {
            content: cleaned,
            stopped_at,
        }
    }
    
    /// Truncate text at the first stop sequence found
    fn truncate_at_stop_sequence(
        text: &mut String,
        stop_sequences: &[String],
        eos_token: &str,
    ) -> Option<String> {
        // Check all stop sequences first, then eos_token - avoid creating string
        for stop_seq in stop_sequences {
            if let Some(pos) = text.find(stop_seq.as_str()) {
                text.truncate(pos);
                return Some(stop_seq.clone());
            }
        }
        // Check eos_token without allocation
        if let Some(pos) = text.find(eos_token) {
            text.truncate(pos);
            return Some(eos_token.to_string());
        }
        None
    }
    
    /// Remove template prefixes/suffixes if echoed by model
    fn remove_template_echoes(text: &mut String, template: &TemplateConfig) {
        // This happens when the model includes its own role markers in the output
        if !template.assistant_prefix.is_empty() && text.starts_with(&template.assistant_prefix) {
            text.drain(..template.assistant_prefix.len());
        }
        if !template.assistant_suffix.is_empty() && text.ends_with(&template.assistant_suffix) {
            let suffix_len = template.assistant_suffix.len();
            let new_len = text.len() - suffix_len;
            text.truncate(new_len);
        }
    }
    
    /// Remove leaked template markers
    fn remove_template_markers(text: &mut String) {
        // Only do replacement if markers are actually present (optimization)
        for marker in TEMPLATE_MARKERS {
            if !marker.is_empty() && text.contains(marker) {
                *text = text.replace(marker, "");
            }
        }
    }
    
    /// Remove role pollution from response
    fn remove_role_pollution(text: &str) -> String {
        // Quick check for dialogue pattern
        if Self::has_dialogue_pattern(text) {
            return ROLE_POLLUTION_FALLBACK.to_string();
        }
        
        let lines: Vec<&str> = text.lines().collect();
        let mut cleaned_lines = Vec::with_capacity(lines.len());
        
        for line in lines {
            if let Some(cleaned) = Self::clean_role_from_line(line) {
                if !cleaned.is_empty() {
                    cleaned_lines.push(cleaned);
                }
            }
        }
        
        let result = cleaned_lines.join("\n").trim().to_string();
        
        // If we've removed everything, return a safe response
        if result.is_empty() {
            EMPTY_RESPONSE_FALLBACK.to_string()
        } else {
            result
        }
    }
    
    /// Check if text contains dialogue pattern
    fn has_dialogue_pattern(text: &str) -> bool {
        text.contains("AI:") && text.contains("You:")
    }
    
    /// Clean role markers from a single line
    fn clean_role_from_line(line: &str) -> Option<String> {
        let trimmed = line.trim_start();
        
        // Check if line starts with any role pattern
        for pattern in ROLE_PATTERNS {
            if trimmed.starts_with(pattern) {
                let remainder = trimmed
                    .trim_start_matches(pattern)
                    .trim();
                
                // If there's actual content after the role marker, return it
                if !remainder.is_empty() {
                    return Some(remainder.to_string());
                } else {
                    // Empty after role marker, skip this line
                    return None;
                }
            }
        }
        
        // Line doesn't start with role pattern, check for mid-line markers
        let mut cleaned = line.to_string();
        for pattern in ROLE_PATTERNS {
            if cleaned.contains(pattern) && !cleaned.starts_with(pattern) {
                // Only remove if it looks like an obvious role marker
                cleaned = cleaned.replace(&format!("\n{}", pattern), "\n");
                cleaned = cleaned.replace(&format!(". {}", pattern), ". ");
            }
        }
        
        Some(cleaned)
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
    /// 
    /// The buffer maintains ALL accumulated content until a stop sequence is found.
    /// Partial emissions are just for incremental display - the buffer keeps everything.
    pub fn process_stream_chunk(
        chunk: &str,
        template: &TemplateConfig,
        stop_sequences: &[String],
        eos_token: &str,
        buffer: &mut String,
    ) -> StreamChunkResult {
        // Add new chunk to accumulated buffer
        let start_len = buffer.len();
        buffer.push_str(chunk);
        
        // Check for stop sequences in entire buffer
        if let Some(stop_seq) = Self::contains_stop_sequence(buffer, stop_sequences, eos_token) {
            // Found stop sequence - clean and finalize the ENTIRE accumulated response
            let cleaned = Self::clean_response(buffer, template, stop_sequences, eos_token);
            let result = StreamChunkResult::Complete {
                content: cleaned.content,
                stopped_at: Some(stop_seq),
            };
            buffer.clear();
            return result;
        }
        
        // No stop sequence yet - emit only the new content if it's safe
        let new_content = &buffer[start_len..];
        
        // For simplicity in streaming, just emit the new content after basic cleaning
        if !new_content.is_empty() {
            let cleaned = Self::remove_role_pollution(new_content);
            StreamChunkResult::Partial {
                content: cleaned,
            }
        } else {
            StreamChunkResult::Buffering
        }
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
                // remove_role_pollution trims the content
                assert_eq!(content, "Hello");
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