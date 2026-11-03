use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{de::DeserializeOwned, Deserialize};
use std::collections::HashMap;

pub mod bindings {
    wit_bindgen::generate!({ generate_all });
}

use crate::bindings::betty_blocks_types::auth::validator::ValidateError;
use crate::bindings::exports::betty_blocks_types::auth::jwt::{AuthError, Guest};

type Headers = Vec<(String, Vec<u8>)>;

struct Component;

const CONFIG_KEY_AUTHENTICATION_PROFILES: &str = "authentication_profiles";
const CONFIG_KEY_ACTIONS: &str = "actions";
const CONFIG_KEY_MCPS: &str = "mcps";

#[derive(Debug, Deserialize)]
struct JwtPayload {
    auth_profile_id: String,
}

/// The claims of a token that the data-api validated.
#[derive(Debug, Deserialize)]
struct DataApiClaims {
    auth_profile: String,
}

/// Validates a token with the data-api, returns the claims as json.
type ValidateJwt<'a> = &'a dyn Fn(&str, &str) -> Result<String, ValidateError>;

type AuthProfileConfig = HashMap<String, String>;

#[derive(Deserialize)]
enum Access {
    Public,
    Private(String),
}

/// How the token of an auth profile is validated.
#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AuthKind {
    /// signed with the secret of the auth profile, validated by this component
    #[default]
    ApiKey,
    /// the token of an end user, validated by the data-api
    DataApi,
}

#[derive(Deserialize)]
struct ResourceAuthConfig {
    /// the auth profile configured for this resource, None means that it is a public resource
    #[serde(rename = "authentication-profile-id")]
    authentication_profile_id: Option<String>,
    /// absent in configs compiled before the data-api kind existed, those are api keys
    #[serde(default)]
    kind: AuthKind,
}

impl ResourceAuthConfig {
    fn as_access(&self) -> Access {
        match self.authentication_profile_id {
            Some(ref auth_id) => Access::Private(auth_id.clone()),
            None => Access::Public,
        }
    }
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

    let decoded_secret = match STANDARD.decode(secret) {
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
        .get(&jwt_profile_id)
        .ok_or_else(|| AuthError::ValidationFailed("Unknown auth profile in JWT".into()))?;
    validate_jwt(&token, profile)?;
    Ok(jwt_profile_id)
}

fn fetch_profile_from_data_api(
    headers: &[(String, Vec<u8>)],
    application_id: &str,
    validate_jwt: ValidateJwt,
) -> Result<String, AuthError> {
    let token = extract_bearer_token(headers)?;
    let claims = validate_jwt(application_id, &token).map_err(|e| match e {
        ValidateError::Invalid(msg) => AuthError::ValidationFailed(msg),
        ValidateError::Unavailable(msg) => AuthError::Unavailable(msg),
    })?;
    let claims: DataApiClaims = serde_json::from_str(&claims).map_err(|e| {
        AuthError::ValidationFailed(format!("Missing auth profile in validated claims: {e}"))
    })?;
    Ok(claims.auth_profile)
}

fn check_profile_authorization(
    headers: &Headers,
    application_id: &str,
    resource_cfg: &ResourceAuthConfig,
    error_msg: &'static str,
    validate_jwt: ValidateJwt,
) -> Result<(), AuthError> {
    match resource_cfg.as_access() {
        Access::Private(auth_profile_id) => {
            let jwt_profile_id = match resource_cfg.kind {
                AuthKind::ApiKey => fetch_validated_profile(headers)?,
                AuthKind::DataApi => {
                    fetch_profile_from_data_api(headers, application_id, validate_jwt)?
                }
            };
            if jwt_profile_id != auth_profile_id {
                return Err(AuthError::Forbidden(error_msg.into()));
            }
            Ok(())
        }
        Access::Public => Ok(()),
    }
}

impl Guest for Component {
    fn allowed_to_call(
        headers: Headers,
        application_id: String,
        action_id: String,
    ) -> Result<(), AuthError> {
        let actions = load_actions_config()?;
        let action_cfg = actions
            .get(&action_id)
            .ok_or_else(|| AuthError::ValidationFailed("Action not found in auth config".into()))?;

        check_profile_authorization(
            &headers,
            &application_id,
            action_cfg,
            "Forbidden: auth profile does not allow this action",
            &crate::bindings::betty_blocks_types::auth::validator::validate_jwt,
        )
    }

    fn allowed_to_list(
        headers: Headers,
        application_id: String,
        mcp_id: String,
    ) -> Result<(), AuthError> {
        let mcps = load_mcps_config()?;
        let mcp_cfg = mcps
            .get(&mcp_id)
            .ok_or_else(|| AuthError::ValidationFailed("MCP not found in auth config".into()))?;

        check_profile_authorization(
            &headers,
            &application_id,
            mcp_cfg,
            "Forbidden: auth profile does not allow to list this mcp server",
            &crate::bindings::betty_blocks_types::auth::validator::validate_jwt,
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

    const APP_ID: &str = "app-id";

    fn api_key_cfg(profile: Option<&str>) -> ResourceAuthConfig {
        ResourceAuthConfig {
            authentication_profile_id: profile.map(str::to_string),
            kind: AuthKind::ApiKey,
        }
    }

    fn data_api_cfg(profile: &str) -> ResourceAuthConfig {
        ResourceAuthConfig {
            authentication_profile_id: Some(profile.to_string()),
            kind: AuthKind::DataApi,
        }
    }

    /// An api key must never be sent to the data-api.
    fn no_data_api(_: &str, _: &str) -> Result<String, ValidateError> {
        panic!("the data-api must not be called")
    }

    fn check(
        headers: &Headers,
        cfg: &ResourceAuthConfig,
        error_msg: &'static str,
        validate: ValidateJwt,
    ) -> Result<(), AuthError> {
        check_profile_authorization(headers, APP_ID, cfg, error_msg, validate)
    }

    #[test]
    fn test_check_profile_authorization_works() {
        let token = make_jwt_token(b"secret", "profile-abc", 3600);
        let headers = make_auth_headers(&token);

        check(
            &headers,
            &api_key_cfg(Some("profile-abc")),
            "",
            &no_data_api,
        )
        .unwrap();
    }

    #[test]
    fn test_check_profile_authorization_auth_profile_not_found() {
        let token = make_jwt_token(b"secret", "profile-xyz", 3600);
        let headers = make_auth_headers(&token);

        let actual = check(
            &headers,
            &api_key_cfg(Some("profile-abc")),
            "",
            &no_data_api,
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
        let actual = check(
            &headers,
            &api_key_cfg(Some("profile-xyz")),
            error_msg,
            &no_data_api,
        )
        .unwrap_err();
        let expected = AuthError::Forbidden(error_msg.to_string());
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn test_check_profile_authorization_public() {
        let headers = make_auth_headers("testing");

        check(&headers, &api_key_cfg(None), "", &no_data_api).unwrap()
    }

    #[test]
    fn test_check_profile_authorization_public_without_token() {
        check(&vec![], &api_key_cfg(None), "", &no_data_api).unwrap()
    }

    #[test]
    fn test_config_without_kind_is_an_api_key() {
        let cfg: ResourceAuthConfig =
            serde_json::from_str(r#"{"authentication-profile-id": "profile-abc"}"#).unwrap();
        assert_eq!(cfg.kind, AuthKind::ApiKey);
    }

    #[test]
    fn test_config_with_data_api_kind() {
        let cfg: ResourceAuthConfig = serde_json::from_str(
            r#"{"authentication-profile-id": "profile-abc", "kind": "data-api", "resource-metadata-url": "https://example.com"}"#,
        )
        .unwrap();
        assert_eq!(cfg.kind, AuthKind::DataApi);
    }

    #[test]
    fn test_config_with_api_key_kind_and_no_profile() {
        let cfg: ResourceAuthConfig =
            serde_json::from_str(r#"{"authentication-profile-id": null}"#).unwrap();
        assert!(matches!(cfg.as_access(), Access::Public));
    }

    #[test]
    fn test_data_api_works() {
        let headers = make_auth_headers("user-token");
        let validate = |app_id: &str, jwt: &str| {
            assert_eq!(app_id, APP_ID);
            assert_eq!(jwt, "user-token");
            Ok(r#"{"user_id": 1, "auth_profile": "profile-abc"}"#.to_string())
        };

        check(&headers, &data_api_cfg("profile-abc"), "", &validate).unwrap();
    }

    #[test]
    fn test_data_api_wrong_profile_is_forbidden() {
        let headers = make_auth_headers("user-token");
        let validate = |_: &str, _: &str| Ok(r#"{"auth_profile": "profile-xyz"}"#.to_string());

        let actual = check(&headers, &data_api_cfg("profile-abc"), "nope", &validate).unwrap_err();
        assert!(matches!(actual, AuthError::Forbidden(msg) if msg == "nope"));
    }

    #[test]
    fn test_data_api_invalid_token() {
        let headers = make_auth_headers("user-token");
        let validate = |_: &str, _: &str| Err(ValidateError::Invalid("expired".to_string()));

        let actual = check(&headers, &data_api_cfg("profile-abc"), "", &validate).unwrap_err();
        assert!(matches!(actual, AuthError::ValidationFailed(msg) if msg == "expired"));
    }

    #[test]
    fn test_data_api_unavailable() {
        let headers = make_auth_headers("user-token");
        let validate = |_: &str, _: &str| Err(ValidateError::Unavailable("down".to_string()));

        let actual = check(&headers, &data_api_cfg("profile-abc"), "", &validate).unwrap_err();
        assert!(matches!(actual, AuthError::Unavailable(msg) if msg == "down"));
    }

    #[test]
    fn test_data_api_claims_without_auth_profile() {
        let headers = make_auth_headers("user-token");
        let validate = |_: &str, _: &str| Ok(r#"{"user_id": 1}"#.to_string());

        let actual = check(&headers, &data_api_cfg("profile-abc"), "", &validate).unwrap_err();
        assert!(matches!(actual, AuthError::ValidationFailed(_)));
    }

    #[test]
    fn test_data_api_without_token_is_malformed() {
        let actual = check(&vec![], &data_api_cfg("profile-abc"), "", &no_data_api).unwrap_err();
        assert!(matches!(actual, AuthError::MalformedToken));
    }
}
