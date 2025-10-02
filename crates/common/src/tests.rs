#[cfg(test)]
mod tests {
    use crate::dto::*;
    use crate::error::Error;

    #[test]
    fn test_message_validation() {
        // Valid message
        let msg = Message {
            role: Role::User,
            content: "Hello".to_string(),
        };
        assert!(msg.validate().is_ok());

        // Empty content
        let msg = Message {
            role: Role::User,
            content: "".to_string(),
        };
        assert!(matches!(msg.validate(), Err(Error::BadRequest(_))));

        // Too long content
        let msg = Message {
            role: Role::User,
            content: "x".repeat(100_001),
        };
        assert!(matches!(msg.validate(), Err(Error::BadRequest(_))));
    }

    #[test]
    fn test_request_validation() {
        // Valid request
        let req = ChatCompletionRequest {
            model: Some("test".to_string()),
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: Some(1.0),
            max_tokens: Some(100),
            stream: Some(false),
            top_p: Some(0.9),
            top_k: Some(40),
            repeat_penalty: Some(1.1),
        };
        assert!(req.validate().is_ok());

        // Empty messages
        let req = ChatCompletionRequest {
            model: None,
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: None,
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };
        assert!(matches!(req.validate(), Err(Error::BadRequest(_))));

        // Invalid temperature
        let req = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: Some(3.0), // Too high
            max_tokens: None,
            stream: None,
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };
        assert!(matches!(req.validate(), Err(Error::BadRequest(_))));

        // Invalid max_tokens
        let req = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: None,
            max_tokens: Some(5000), // Too high
            stream: None,
            top_p: None,
            top_k: None,
            repeat_penalty: None,
        };
        assert!(matches!(req.validate(), Err(Error::BadRequest(_))));

        // Invalid top_p
        let req = ChatCompletionRequest {
            model: None,
            messages: vec![Message {
                role: Role::User,
                content: "Hello".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stream: None,
            top_p: Some(1.5), // Too high
            top_k: None,
            repeat_penalty: None,
        };
        assert!(matches!(req.validate(), Err(Error::BadRequest(_))));
    }

    #[test]
    fn test_role_conversion() {
        assert_eq!(Role::from("system".to_string()), Role::System);
        assert_eq!(Role::from("SYSTEM".to_string()), Role::System);
        assert_eq!(Role::from("user".to_string()), Role::User);
        assert_eq!(Role::from("USER".to_string()), Role::User);
        assert_eq!(Role::from("assistant".to_string()), Role::Assistant);
        assert_eq!(Role::from("ASSISTANT".to_string()), Role::Assistant);
        assert_eq!(Role::from("unknown".to_string()), Role::User); // Default
    }

    #[test]
    fn test_error_status_codes() {
        assert_eq!(Error::BadRequest("test".into()).status_code(), 400);
        assert_eq!(Error::ModelNotFound("test".into()).status_code(), 404);
        assert_eq!(Error::RateLimitExceeded.status_code(), 429);
        assert_eq!(Error::ServiceUnavailable("test".into()).status_code(), 503);
        assert_eq!(Error::Timeout(30).status_code(), 408);
        assert_eq!(Error::UserCancelled.status_code(), 499);
        assert_eq!(Error::Internal("test".into()).status_code(), 500);
    }

    #[test]
    fn test_error_retryable() {
        assert!(Error::ServiceUnavailable("test".into()).is_retryable());
        assert!(Error::RuntimeNotReady.is_retryable());
        assert!(Error::Timeout(30).is_retryable());
        assert!(!Error::BadRequest("test".into()).is_retryable());
        assert!(!Error::ModelNotFound("test".into()).is_retryable());
    }

    #[test]
    fn test_generation_params_from_request() {
        let req = ChatCompletionRequest {
            model: None,
            messages: vec![],
            temperature: Some(0.8),
            max_tokens: Some(200),
            stream: None,
            top_p: Some(0.95),
            top_k: Some(50),
            repeat_penalty: Some(1.2),
        };

        let defaults = GenerationParams::default();
        let params = GenerationParams::from_request(&req, defaults);

        assert_eq!(params.temperature, 0.8);
        assert_eq!(params.max_tokens, 200);
        assert_eq!(params.top_p, 0.95);
        assert_eq!(params.top_k, 50);
        assert_eq!(params.repeat_penalty, 1.2);
        assert!(!params.request_id.is_empty());
    }
}
