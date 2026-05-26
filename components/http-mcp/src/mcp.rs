use crate::actions;
use crate::betty_blocks::auth::jwt::{allowed_to_call, allowed_to_list, AuthError};
use crate::config;
use crate::types::McpServerConfig;
use rust_mcp_schema::{
    CallToolRequestParams, CallToolResult, JsonrpcErrorResponse, JsonrpcRequest, JsonrpcResponse,
    JsonrpcResultResponse, ListToolsResult, RequestId, RpcError,
};
use serde_json::Value;

type Headers = Vec<(String, Vec<u8>)>;

#[derive(serde::Deserialize)]
struct JsonrpcRequestWithoutId {
    pub method: String,
}

pub async fn process_rpc(
    server_id: &str,
    body: &str,
    headers: &Headers,
    wasmcloud_host: &str,
    application_id: &str,
) -> Result<Option<JsonrpcResponse>, JsonrpcErrorResponse> {
    match serde_json::from_str::<JsonrpcRequestWithoutId>(body) {
        // short circuit notifications
        Ok(r) if r.method.starts_with("notifications/") => {
            return Ok(None);
        }
        _ => (),
    };

    let request_obj: JsonrpcRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return Err(create_error_response(
                -32700,
                &format!("Invalid JSON-RPC request: {e}"),
                None,
            ));
        }
    };

    let id = Some(request_obj.id);

    let server_config = match config::load_server_config(server_id) {
        Ok(cfg) => cfg,
        Err(e) => {
            return Err(create_error_response(
                -32000,
                &format!("Failed to load server config: {e}"),
                id,
            ));
        }
    };

    let params = match request_obj.params {
        Some(p) => Some(serde_json::to_value(p).map_err(|e| {
            create_error_response(-32600, &format!("Invalid params: {e}"), id.clone())
        })?),
        None => None,
    };

    let result = match request_obj.method.as_str() {
        "initialize" => handle_initialize(headers, &server_config),
        "tools/list" => handle_list_tools(headers, &server_config),
        "tools/call" => {
            handle_call_tool(
                &server_config,
                headers,
                params,
                wasmcloud_host,
                application_id,
            )
            .await
        }
        _ => {
            return Err(create_error_response(
                -32601,
                &format!("Method not found: {}", request_obj.method),
                id,
            ));
        }
    };

    match result {
        Ok(result_value) => match create_success_response(id, result_value) {
            Ok(resp) => Ok(Some(resp)),
            Err(e) => Err(create_error_response(
                -32603,
                &format!("Failed to build response: {e}"),
                None,
            )),
        },
        Err(e) => Err(create_error_response(-32000, &e, id)),
    }
}

fn check_allowed_to_list(headers: &Headers, server_id: &str) -> Result<(), String> {
    if let Err(e) = allowed_to_list(headers, server_id) {
        return Err(format!("Auth error: {}", auth_error_message(e)));
    }

    Ok(())
}

fn handle_initialize(headers: &Headers, server_config: &McpServerConfig) -> Result<Value, String> {
    check_allowed_to_list(headers, &server_config.id)?;
    let result = crate::config::build_initialize_result(server_config);
    serde_json::to_value(result).map_err(|e| format!("Failed to serialize initialize result: {e}"))
}

fn handle_list_tools(headers: &Headers, server_config: &McpServerConfig) -> Result<Value, String> {
    check_allowed_to_list(headers, &server_config.id)?;
    let result = ListToolsResult {
        tools: server_config.tools.iter().map(|t| t.tool.clone()).collect(),
        next_cursor: None,
        meta: None,
    };

    serde_json::to_value(result).map_err(|e| format!("Failed to serialize tools list: {e}"))
}

async fn handle_call_tool(
    server_config: &McpServerConfig,
    headers: &Headers,
    params: Option<Value>,
    wasmcloud_host: &str,
    application_id: &str,
) -> Result<Value, String> {
    let params = params.ok_or("Missing params for tools/call")?;
    let call_params: CallToolRequestParams =
        serde_json::from_value(params).map_err(|e| format!("Invalid tool call parameters: {e}"))?;

    let tool_with_action = server_config
        .tools
        .iter()
        .find(|t| t.tool.name == call_params.name)
        .ok_or_else(|| format!("Tool '{}' not found", call_params.name))?;

    crate::validation::validate_arguments(
        call_params.arguments.as_ref(),
        &tool_with_action.tool.input_schema,
    )?;

    if let Err(e) = allowed_to_call(headers, &tool_with_action.action_id) {
        return Err(format!("Auth error: {}", auth_error_message(e)));
    }

    let configurations = config::load_betty_config().unwrap_or(String::from("{}"));

    let args_value = call_params
        .arguments
        .as_ref()
        .map(|m| Value::Object(m.clone()))
        .unwrap_or(Value::Null);

    let (is_error, content) = actions::execute_mapped_action(
        &tool_with_action.action_id,
        &args_value,
        &configurations,
        wasmcloud_host,
        application_id,
    )
    .await?;

    let result = CallToolResult {
        content,
        structured_content: None,
        is_error: Some(is_error),
        meta: None,
    };

    serde_json::to_value(result).map_err(|e| format!("Failed to serialize tool result: {e}"))
}

pub fn create_success_response(
    id: Option<RequestId>,
    result: Value,
) -> Result<JsonrpcResponse, String> {
    let request_id = id.ok_or("id must be a string or integer")?;
    let mcp_result = match result {
        Value::Object(map) => rust_mcp_schema::Result {
            meta: None,
            extra: Some(map),
        },
        _ => return Err("result must be a JSON object".to_string()),
    };
    Ok(JsonrpcResponse::from(JsonrpcResultResponse::new(
        request_id, mcp_result,
    )))
}

pub fn create_error_response(
    code: i32,
    message: &str,
    id: Option<RequestId>,
) -> JsonrpcErrorResponse {
    JsonrpcErrorResponse::new(
        RpcError {
            code: code as i64,
            message: message.to_string(),
            data: None,
        },
        id,
    )
}

fn auth_error_message(e: AuthError) -> String {
    match e {
        AuthError::MalformedToken => "malformed token".to_string(),
        AuthError::MissingConfig(msg) | AuthError::ValidationFailed(msg) => msg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_mcp_schema::JSONRPC_VERSION;
    use serde_json::json;

    fn make_test_server_config() -> McpServerConfig {
        serde_json::from_value(json!({
            "id": "test-server",
            "tools": [
                {
                    "name": "get_weather",
                    "description": "Get weather for a location",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "location": { "type": "string" }
                        },
                        "required": ["location"]
                    },
                    "action-id": "action-001"
                },
                {
                    "name": "add_numbers",
                    "description": "Add two numbers",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "a": { "type": "number" },
                            "b": { "type": "number" }
                        },
                        "required": ["a", "b"]
                    },
                    "action-id": "action-002"
                }
            ]
        }))
        .expect("test server config must parse")
    }

    #[tokio::test]
    async fn test_notifications_initialized_returns_no_content() {
        let body = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let headers: Headers = vec![("content-type".to_string(), b"application/json".to_vec())];
        let result = process_rpc("any-server", body, &headers, "host", "app-id").await;
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn test_create_success_response_with_numeric_id() {
        let result = create_success_response(Some(RequestId::Integer(1)), json!({"status": "ok"}));
        assert!(result.is_ok());

        let serialized = serde_json::to_value(result.unwrap()).unwrap();
        assert_eq!(serialized["jsonrpc"], JSONRPC_VERSION);
        assert_eq!(serialized["id"], json!(1));
        assert_eq!(serialized["result"]["status"], "ok");
    }

    #[test]
    fn test_create_success_response_with_string_id() {
        let result = create_success_response(
            Some(RequestId::String("req-42".to_string())),
            json!({"data": [1, 2, 3]}),
        );
        assert!(result.is_ok());

        let serialized = serde_json::to_value(result.unwrap()).unwrap();
        assert_eq!(serialized["id"], "req-42");
    }

    #[test]
    fn test_create_success_response_with_null_id_fails() {
        let result = create_success_response(None, json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_error_response_method_not_found() {
        let resp = create_error_response(-32601, "Method not found", Some(RequestId::Integer(1)));

        let serialized = serde_json::to_value(&resp).unwrap();
        assert_eq!(serialized["jsonrpc"], JSONRPC_VERSION);
        assert_eq!(serialized["id"], 1);
        assert_eq!(serialized["error"]["code"], -32601);
        assert_eq!(serialized["error"]["message"], "Method not found");
    }

    #[test]
    fn test_create_error_response_parse_error() {
        let resp = create_error_response(-32700, "Parse error", None);

        let serialized = serde_json::to_value(&resp).unwrap();
        assert!(serialized["id"].is_null());
        assert_eq!(serialized["error"]["code"], -32700);
    }

    #[test]
    fn test_create_error_response_invalid_request() {
        let resp = create_error_response(
            -32600,
            "Invalid Request",
            Some(RequestId::String("abc".to_string())),
        );

        let serialized = serde_json::to_value(&resp).unwrap();
        assert_eq!(serialized["error"]["code"], -32600);
        assert_eq!(serialized["error"]["message"], "Invalid Request");
    }

    #[test]
    fn test_create_error_response_internal_error() {
        let _resp = create_error_response(-32603, "Internal error", Some(RequestId::Integer(99)));
    }

    #[test]
    fn test_handle_call_tool_unknown_tool() {
        let config = make_test_server_config();
        let tool = config
            .tools
            .iter()
            .find(|t| t.tool.name == "nonexistent_tool");
        assert!(tool.is_none());
    }

    #[test]
    fn test_handle_call_tool_invalid_params() {
        let call_params: Result<CallToolRequestParams, _> =
            serde_json::from_value(json!({ "invalid": true }));
        assert!(call_params.is_err(), "invalid params should fail to parse");
    }

    #[tokio::test]
    async fn test_handle_call_tool_missing_params() {
        let config = make_test_server_config();
        let headers: Headers = vec![];
        let result = handle_call_tool(&config, &headers, None, "host", "app-id").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Missing params for tools/call");
    }

    #[tokio::test]
    async fn test_handle_call_tool_empty_object_params() {
        let config = make_test_server_config();
        let headers: Headers = vec![];
        let result = handle_call_tool(&config, &headers, Some(json!({})), "host", "app-id").await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("Invalid tool call parameters"),
            "empty object should fail because 'name' is required"
        );
    }

    #[test]
    fn test_call_tool_params_name_only() {
        let call_params: CallToolRequestParams =
            serde_json::from_value(json!({ "name": "some_tool" })).unwrap();
        assert_eq!(call_params.name, "some_tool");
        assert!(
            call_params.arguments.is_none(),
            "arguments should be None when omitted"
        );
    }

    #[test]
    fn test_call_tool_params_with_arguments() {
        let call_params: CallToolRequestParams = serde_json::from_value(json!({
            "name": "some_tool",
            "arguments": { "key": "value", "count": 42 }
        }))
        .unwrap();
        assert_eq!(call_params.name, "some_tool");
        let args = call_params.arguments.unwrap();
        assert_eq!(args["key"], "value");
        assert_eq!(args["count"], 42);
    }

    #[test]
    fn test_call_tool_params_without_arguments() {
        let call_params: CallToolRequestParams = serde_json::from_value(json!({
            "name": "some_tool",
            "arguments": {}
        }))
        .unwrap();
        assert_eq!(call_params.name, "some_tool");
        assert!(
            call_params.arguments.unwrap().is_empty(),
            "arguments should be an empty map when passed as {{}}"
        );
    }

    #[test]
    fn test_success_response_round_trip() {
        let original_result = json!({
            "tools": [{"name": "test", "inputSchema": {"type": "object"}}]
        });
        let resp =
            create_success_response(Some(RequestId::Integer(1)), original_result.clone()).unwrap();
        let serialized = serde_json::to_string(&resp).unwrap();
        let deserialized: Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized["jsonrpc"], JSONRPC_VERSION);
        assert_eq!(deserialized["id"], 1);
        assert_eq!(deserialized["result"], original_result);
    }

    #[test]
    fn test_error_response_round_trip() {
        let resp = create_error_response(-32601, "Method not found", Some(RequestId::Integer(42)));
        let serialized = serde_json::to_string(&resp).unwrap();
        let deserialized: Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized["jsonrpc"], JSONRPC_VERSION);
        assert_eq!(deserialized["id"], 42);
        assert_eq!(deserialized["error"]["code"], -32601);
        assert_eq!(deserialized["error"]["message"], "Method not found");
    }
}
