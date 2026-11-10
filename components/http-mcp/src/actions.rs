use action_client::{ActionError, ActionRequest};
use rust_mcp_schema::ContentBlock;
use serde_json::Value;

pub async fn execute_mapped_action(
    action_id: &str,
    arguments: &Value,
    configurations: &str,
    wasmcloud_host: &str,
    application_id: &str,
    jwt: Option<&str>,
) -> Result<(bool, Vec<ContentBlock>), String> {
    let action_request = ActionRequest {
        wasmcloud_host,
        application_id,
        action_id,
        input: arguments,
        configurations,
        jwt,
    };
    match action_client::call_action(&action_request).await {
        Ok(output) => Ok((false, parse_action_output(&output))),
        Err(ActionError::Failed(e)) => Err(e),
        Err(e) => Ok((true, vec![ContentBlock::text_content(e.to_string())])),
    }
}

pub fn parse_action_output(output: &Value) -> Vec<ContentBlock> {
    match output {
        Value::Null => vec![],
        Value::String(s) => vec![ContentBlock::text_content(s.clone())],
        other => vec![ContentBlock::text_content(
            serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_action_output_null() {
        let content = parse_action_output(&Value::Null);
        assert!(
            content.is_empty(),
            "null output should produce no content blocks"
        );
    }

    #[test]
    fn test_parse_action_output_string() {
        let content = parse_action_output(&json!("Hello, world!"));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => assert_eq!(t.text, "Hello, world!"),
            _ => panic!("expected TextContent"),
        }
    }

    #[test]
    fn test_parse_action_output_object() {
        let content = parse_action_output(&json!({"key": "value", "num": 42}));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => {
                assert!(t.text.contains("key"));
                assert!(t.text.contains("value"));
            }
            _ => panic!("expected TextContent"),
        }
    }

    #[test]
    fn test_parse_action_output_number() {
        let content = parse_action_output(&json!(42));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => assert_eq!(t.text, "42"),
            _ => panic!("expected TextContent"),
        }
    }

    #[test]
    fn test_parse_action_output_array() {
        let content = parse_action_output(&json!([1, 2, 3]));
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::TextContent(t) => {
                assert!(t.text.contains('1'));
                assert!(t.text.contains('3'));
            }
            _ => panic!("expected TextContent"),
        }
    }
}
