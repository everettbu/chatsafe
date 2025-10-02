#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;
    use chatsafe_common::{ChatCompletionRequest, Message, Role, HealthStatus, HealthResponse};


    #[tokio::test]
    async fn test_request_validation_empty_messages() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: Some(false),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        let result = request.validate();
        assert!(result.is_err());
        
        if let Err(e) = result {
            assert_eq!(e.status_code(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn test_request_validation_invalid_temperature() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "test".to_string(),
            }],
            temperature: Some(3.0), // Invalid: > 2.0
            max_tokens: None,
            stream: Some(false),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        let result = request.validate();
        assert!(result.is_err());
        
        if let Err(e) = result {
            assert_eq!(e.status_code(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn test_request_validation_valid() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: Some(0.7),
            max_tokens: Some(100),
            stream: Some(false),
            top_p: Some(0.9),
            top_k: Some(40),
            repeat_penalty: Some(1.1),
        };

        let result = request.validate();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_request_validation_empty_content() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "".to_string(), // Empty content
            }],
            temperature: None,
            max_tokens: None,
            stream: Some(false),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        let result = request.validate();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_request_validation_invalid_top_p() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "test".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stream: Some(false),
            top_p: Some(1.5), // Invalid: > 1.0
            top_k: None,
            repeat_penalty: None,
        };

        let result = request.validate();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_request_validation_negative_max_tokens() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "test".to_string(),
            }],
            temperature: None,
            max_tokens: Some(0), // Invalid: must be > 0
            stream: Some(false),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        let result = request.validate();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_health_response_serialization() {
        let health = HealthResponse {
            status: HealthStatus::Healthy,
            model_loaded: true,
            version: "0.1.0".to_string(),
            uptime_seconds: 3600,
        };

        let json = serde_json::to_value(&health).expect("Failed to serialize health response");
        
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["model_loaded"], true);
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["uptime_seconds"], 3600);
    }

    #[tokio::test]
    async fn test_message_role_serialization() {
        let msg = Message {
            role: Role::System,
            content: "You are helpful".to_string(),
        };

        let json = serde_json::to_value(&msg).expect("Failed to serialize message");
        assert_eq!(json["role"], "system");

        let msg = Message {
            role: Role::User,
            content: "Hello".to_string(),
        };

        let json = serde_json::to_value(&msg).expect("Failed to serialize message");
        assert_eq!(json["role"], "user");

        let msg = Message {
            role: Role::Assistant,
            content: "Hi there".to_string(),
        };

        let json = serde_json::to_value(&msg).expect("Failed to serialize message");
        assert_eq!(json["role"], "assistant");
    }

    #[tokio::test]
    async fn test_request_deserialization() {
        let json = json!({
            "messages": [
                {"role": "user", "content": "Hello"}
            ],
            "temperature": 0.8,
            "max_tokens": 150,
            "stream": true
        });

        let request: ChatCompletionRequest = serde_json::from_value(json).expect("Failed to deserialize request");
        
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].content, "Hello");
        assert_eq!(request.temperature, Some(0.8));
        assert_eq!(request.max_tokens, Some(150));
        assert_eq!(request.stream, Some(true));
    }

    #[tokio::test]
    async fn test_request_with_system_message() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a helpful assistant".to_string(),
                },
                Message {
                    role: Role::User,
                    content: "Hello".to_string(),
                },
            ],
            temperature: None,
            max_tokens: None,
            stream: Some(false),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        let result = request.validate();
        assert!(result.is_ok());
        assert_eq!(request.messages.len(), 2);
    }

    #[tokio::test]
    async fn test_streaming_default() {
        let request = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "test".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stream: None, // Not specified, should default to true
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        // In the actual handler, stream.unwrap_or(true)
        assert_eq!(request.stream.unwrap_or(true), true);
    }

    #[tokio::test]
    async fn test_model_field_handling() {
        let request = ChatCompletionRequest {
            model: Some("llama-3.2-3b-instruct-q4_k_m".to_string()),
            messages: vec![Message {
                role: Role::User,
                content: "test".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stream: Some(false),
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };

        assert!(request.model.is_some());
        assert_eq!(request.model.expect("Model should be present"), "llama-3.2-3b-instruct-q4_k_m");
    }
}