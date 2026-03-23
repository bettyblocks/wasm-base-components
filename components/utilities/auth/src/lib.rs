use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{de::DeserializeOwned, Deserialize};
use std::collections::HashMap;

pub mod bindings {
    wit_bindgen::generate!({ generate_all });
}

use crate::bindings::exports::betty_blocks::auth::jwt::{AuthError, Guest};

type Headers = Vec<(String, Vec<u8>)>;

struct Component;

const CONFIG_KEY_AUTHENTICATION_PROFILES: &str = "authentication_profiles";
const CONFIG_KEY_ACTIONS: &str = "actions";
const CONFIG_KEY_MCPS: &str = "mcps";

#[derive(Debug, Deserialize)]
struct JwtPayload {
    auth_profile_id: String,
}

type AuthProfileConfig = HashMap<String, String>;

#[derive(Deserialize)]
struct ResourceAuthConfig {
    /// the auth profile configured for this resource, None means that it is a public resource
    #[serde(rename = "authentication-profile-id")]
    authentication_profile_id: Option<String>,
}

#[cfg(test)]
fn load_config<T: DeserializeOwned>(key: &str) -> Result<T, AuthError> {
    let auth_profiles: AuthProfileConfig =
        HashMap::from_iter([("profile-abc".to_string(), "c2VjcmV0".to_string())]);
    let map: HashMap<String, String> = HashMap::from_iter([(
        "authentication_profiles".to_string(),
        serde_json::to_string(&auth_profiles).unwrap(),
    )]);
    config_from_string(key, Ok(map.get(key).map(|x| x.to_string())))
}

#[cfg(not(test))]
fn load_config<T: DeserializeOwned>(key: &str) -> Result<T, AuthError> {
    config_from_string(key, crate::bindings::wasi::config::store::get(key))
}

fn config_from_string<T: DeserializeOwned>(
    key: &str,
    input: Result<Option<String>, crate::bindings::wasi::config::store::Error>,
) -> Result<T, AuthError> {
    let raw = input
        .map_err(|e| {
            AuthError::MissingConfig(format!("Config store error for '{}': {:?}", key, e))
        })?
        .ok_or_else(|| {
            AuthError::MissingConfig(format!("Key '{}' not found in config store", key))
        })?;
    serde_json::from_str(&raw)
        .map_err(|e| AuthError::MissingConfig(format!("Failed to parse {}: {}", key, e)))
}

fn load_auth_profiles() -> Result<AuthProfileConfig, AuthError> {
    load_config(CONFIG_KEY_AUTHENTICATION_PROFILES)
}

fn load_actions_config() -> Result<HashMap<String, ResourceAuthConfig>, AuthError> {
    load_config(CONFIG_KEY_ACTIONS)
}

fn load_mcps_config() -> Result<HashMap<String, ResourceAuthConfig>, AuthError> {
    load_config(CONFIG_KEY_MCPS)
}

fn extract_bearer_token(headers: &[(String, Vec<u8>)]) -> Result<String, AuthError> {
    let auth_value = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .ok_or(AuthError::MalformedToken)?;
    let value = String::from_utf8_lossy(&auth_value.1);
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "null")
        .map(str::to_string)
        .ok_or(AuthError::MalformedToken)
}

fn peek_auth_profile_id(token: &str) -> Result<String, AuthError> {
    let payload_b64 = token.split('.').nth(1).ok_or(AuthError::MalformedToken)?;
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| AuthError::MalformedToken)?;
    let claims: JwtPayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| AuthError::MalformedToken)?;
    Ok(claims.auth_profile_id)
}

fn validate_jwt(token: &str, secret: &str) -> Result<(), AuthError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.leeway = 30;
    validation.set_required_spec_claims(&["exp"]);

    let decoded_secret = match URL_SAFE_NO_PAD.decode(secret) {
        Ok(x) => x,
        Err(_) => secret.as_bytes().to_vec(),
    };
    jsonwebtoken::decode::<serde_json::Value>(
        token,
        &DecodingKey::from_secret(&decoded_secret),
        &validation,
    )
    .map_err(|e| AuthError::ValidationFailed(format!("JWT validation failed: {}", e)))?;
    Ok(())
}

fn fetch_validated_profile(headers: &[(String, Vec<u8>)]) -> Result<String, AuthError> {
    let token = extract_bearer_token(headers)?;
    let jwt_profile_id = peek_auth_profile_id(&token)?;
    let profiles = load_auth_profiles()?;
    let profile = profiles
        .get_key_value(&jwt_profile_id)
        .ok_or_else(|| AuthError::ValidationFailed("Unknown auth profile in JWT".into()))?;
    validate_jwt(&token, profile.1)?;
    Ok(jwt_profile_id)
}

fn check_profile_authorization(
    headers: &Headers,
    resource_cfg: &ResourceAuthConfig,
    error_msg: &'static str,
) -> Result<(), AuthError> {
    if let Some(ref auth_profile_id) = resource_cfg.authentication_profile_id {
        let jwt_profile_id = fetch_validated_profile(headers)?;
        if &jwt_profile_id != auth_profile_id {
            return Err(AuthError::ValidationFailed(error_msg.into()));
        }
        Ok(())
    } else {
        // resource is public so allow everyone
        Ok(())
    }
}

impl Guest for Component {
    fn allowed_to_call(headers: Headers, action_id: String) -> Result<(), AuthError> {
        let actions = load_actions_config()?;
        let action_cfg = actions
            .get(&action_id)
            .ok_or_else(|| AuthError::ValidationFailed("Action not found in auth config".into()))?;

        check_profile_authorization(
            &headers,
            action_cfg,
            "Forbidden: auth profile does not allow this action",
        )
    }

    fn allowed_to_list(headers: Headers, mcp_id: String) -> Result<(), AuthError> {
        let mcps = load_mcps_config()?;
        let mcp_cfg = mcps
            .get(&mcp_id)
            .ok_or_else(|| AuthError::ValidationFailed("MCP not found in auth config".into()))?;

        check_profile_authorization(
            &headers,
            mcp_cfg,
            "Forbidden: auth profile does not allow to list this mcp server",
        )
    }
}

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde::Serialize;

    fn make_jwt_token(secret: &[u8], auth_profile: &str, exp_offset: i64) -> String {
        #[derive(Serialize)]
        struct Claims {
            auth_profile_id: String,
            exp: u64,
            nbf: u64,
            iat: u64,
        }
        let n = jsonwebtoken::get_current_timestamp();
        let claims = Claims {
            auth_profile_id: auth_profile.to_string(),
            exp: (n as i64 + exp_offset) as u64,
            nbf: n,
            iat: n,
        };
        let header = Header::new(Algorithm::HS256);
        jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(secret))
            .expect("encode failed")
    }

    fn make_auth_headers(token: &str) -> Vec<(String, Vec<u8>)> {
        vec![(
            "authorization".to_string(),
            format!("Bearer {}", token).into_bytes(),
        )]
    }

    #[test]
    fn test_extract_bearer_token_valid() {
        let headers = make_auth_headers("my-token");
        let result = extract_bearer_token(&headers);
        assert_eq!(result.unwrap(), "my-token");
    }

    #[test]
    fn test_extract_bearer_token_missing() {
        let headers: Vec<(String, Vec<u8>)> = vec![];
        let result = extract_bearer_token(&headers);
        assert!(matches!(result, Err(AuthError::MalformedToken)));
    }

    #[test]
    fn test_extract_bearer_token_no_bearer_prefix() {
        let headers = vec![("authorization".to_string(), b"Basic abc123".to_vec())];
        let result = extract_bearer_token(&headers);
        assert!(matches!(result, Err(AuthError::MalformedToken)));
    }

    #[test]
    fn test_extract_bearer_token_null_value() {
        let headers = make_auth_headers("null");
        let result = extract_bearer_token(&headers);
        assert!(matches!(result, Err(AuthError::MalformedToken)));
    }

    #[test]
    fn test_peek_auth_profile_id_valid() {
        let token = make_jwt_token(b"secret", "profile-abc", 3600);
        let result = peek_auth_profile_id(&token);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "profile-abc");
    }

    #[test]
    fn test_peek_auth_profile_id_malformed_no_dots() {
        let result = peek_auth_profile_id("nodots");
        assert!(matches!(result, Err(AuthError::MalformedToken)));
    }

    #[test]
    fn test_peek_auth_profile_id_invalid_base64() {
        let result = peek_auth_profile_id("header.!!!invalid_base64!!!.sig");
        assert!(matches!(result, Err(AuthError::MalformedToken)));
    }

    #[test]
    fn test_validate_jwt_valid() {
        let secret = b"test_secret";
        let token = make_jwt_token(secret, "profile-xyz", 3600);
        let result = validate_jwt(&token, "test_secret");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_jwt_wrong_secret() {
        let token = make_jwt_token(b"correct_secret", "profile-xyz", 3600);
        let result = validate_jwt(&token, "wrong_secret");
        assert!(matches!(result, Err(AuthError::ValidationFailed(_))));
    }

    #[test]
    fn test_validate_jwt_expired() {
        let secret = b"test_secret";
        let token = make_jwt_token(secret, "profile-xyz", -3600);
        let result = validate_jwt(&token, "test_secret");
        assert!(matches!(result, Err(AuthError::ValidationFailed(_))));
    }

    #[test]
    fn test_check_profile_authorization_works() {
        let token = make_jwt_token(b"secret", "profile-abc", 3600);
        let headers = make_auth_headers(&token);

        check_profile_authorization(
            &headers,
            &ResourceAuthConfig {
                authentication_profile_id: Some("profile-abc".to_string()),
            },
            "",
        )
        .unwrap();
    }

    #[test]
    fn test_check_profile_authorization_auth_profile_not_found() {
        let token = make_jwt_token(b"secret", "profile-xyz", 3600);
        let headers = make_auth_headers(&token);

        let actual = check_profile_authorization(
            &headers,
            &ResourceAuthConfig {
                authentication_profile_id: Some("profile-abc".to_string()),
            },
            "",
        )
        .unwrap_err();
        let expected = AuthError::ValidationFailed("Unknown auth profile in JWT".to_string());
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn test_check_profile_authorization_incorrect_auth_profile() {
        let token = make_jwt_token(b"secret", "profile-abc", 3600);
        let headers = make_auth_headers(&token);

        let error_msg = "here";
        let actual = check_profile_authorization(
            &headers,
            &ResourceAuthConfig {
                authentication_profile_id: Some("profile-xyz".to_string()),
            },
            error_msg,
        )
        .unwrap_err();
        let expected = AuthError::ValidationFailed(error_msg.to_string());
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn test_check_profile_authorization_public() {
        let headers = make_auth_headers("testing");

        check_profile_authorization(
            &headers,
            &ResourceAuthConfig {
                authentication_profile_id: None,
            },
            "",
        )
        .unwrap()
    }
}
