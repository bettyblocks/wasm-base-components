use serde_json::Value;
use wstd::http::{Body, Method, Request, Response, StatusCode};

wit_bindgen::generate!({
    world: "well-known",
    generate_all,
});

mod config;

const DOCUMENTS: [&str; 2] = ["oauth-protected-resource", "oauth-authorization-server"];
const MAX_ID_LENGTH: usize = 64;

#[derive(Debug, PartialEq)]
struct Route {
    document: &'static str,
    app_uuid: String,
    profile_id: String,
}

#[derive(Debug, PartialEq)]
enum Failure {
    NotFound,
    /// something is wrong with the deployment of this component, not with the request
    Internal(String),
}

impl Failure {
    fn into_response(self) -> Response<Body> {
        match self {
            Failure::NotFound => status_response(StatusCode::NOT_FOUND),
            Failure::Internal(message) => {
                eprintln!("well-known: {message}");
                status_response(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

#[wstd::http_server]
async fn main(request: Request<Body>) -> Result<Response<Body>, wstd::http::Error> {
    Ok(inner_handle(&request))
}

fn inner_handle(request: &Request<Body>) -> Response<Body> {
    if request.method() != Method::GET {
        return status_response(StatusCode::METHOD_NOT_ALLOWED);
    }

    let Some(route) = parse_route(request.uri().path()) else {
        return status_response(StatusCode::NOT_FOUND);
    };

    match serve(&route) {
        Ok(document) => json_response(document),
        Err(failure) => failure.into_response(),
    }
}

fn serve(route: &Route) -> Result<String, Failure> {
    let application_id = config::load_application_id().map_err(Failure::Internal)?;
    let config = config::load_config().map_err(Failure::Internal)?;

    let document = find_document(&config, &application_id, route)?;
    serde_json::to_string(document).map_err(|e| Failure::Internal(e.to_string()))
}

/// The document a request asks for, if this application has it.
fn find_document<'a>(
    config: &'a config::WellKnownConfig,
    application_id: &str,
    route: &Route,
) -> Result<&'a Value, Failure> {
    if route.app_uuid != application_id {
        return Err(Failure::NotFound);
    }

    config
        .profiles
        .get(&route.profile_id)
        .and_then(|documents| documents.get(route.document))
        .ok_or(Failure::NotFound)
}

/// Matches `/.well-known/{document}/api/runtime/{app_uuid}/authentication_profile/{profile_id}`
/// for every document. Anything else, including ids with characters outside `[A-Za-z0-9_-]`,
/// is no route.
fn parse_route(path: &str) -> Option<Route> {
    let mut router = matchit::Router::new();
    for document in DOCUMENTS {
        router
            .insert(
                format!(
                    "/.well-known/{document}/api/runtime/{{app_uuid}}/authentication_profile/{{profile_id}}"
                ),
                document,
            )
            .ok()?;
    }

    let matched = router.at(path).ok()?;
    let app_uuid = matched.params.get("app_uuid")?;
    let profile_id = matched.params.get("profile_id")?;
    if !is_valid_id(app_uuid) || !is_valid_id(profile_id) {
        return None;
    }

    Some(Route {
        document: matched.value,
        app_uuid: app_uuid.to_string(),
        profile_id: profile_id.to_string(),
    })
}

fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LENGTH
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn json_response(body: String) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap_or_else(|_| status_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn status_response(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::from(String::new()));
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{parse_config, WellKnownConfig};

    const APP: &str = "b0291e381fe54be98dd6a0611d49fb67";

    fn test_config() -> WellKnownConfig {
        parse_config(
            r#"{
                "well-known": {
                    "profiles": {
                        "profile-1": {
                            "oauth-protected-resource": { "resource": "https://app.betty.app" }
                        }
                    }
                }
            }"#,
        )
        .unwrap()
    }

    fn route(document: &'static str, app_uuid: &str, profile_id: &str) -> Route {
        Route {
            document,
            app_uuid: app_uuid.to_string(),
            profile_id: profile_id.to_string(),
        }
    }

    #[test]
    fn test_parse_route_protected_resource() {
        let path = format!(
            "/.well-known/oauth-protected-resource/api/runtime/{APP}/authentication_profile/profile-1"
        );
        assert_eq!(
            parse_route(&path),
            Some(route("oauth-protected-resource", APP, "profile-1"))
        );
    }

    #[test]
    fn test_parse_route_authorization_server() {
        let path = format!(
            "/.well-known/oauth-authorization-server/api/runtime/{APP}/authentication_profile/profile-1"
        );
        assert_eq!(
            parse_route(&path),
            Some(route("oauth-authorization-server", APP, "profile-1"))
        );
    }

    #[test]
    fn test_parse_route_rejects_unknown_document() {
        let path = format!(
            "/.well-known/openid-configuration/api/runtime/{APP}/authentication_profile/profile-1"
        );
        assert_eq!(parse_route(&path), None);
    }

    #[test]
    fn test_parse_route_rejects_other_paths() {
        assert_eq!(parse_route("/"), None);
        assert_eq!(parse_route("/api/mcp/server-1"), None);
        assert_eq!(parse_route("/.well-known/oauth-protected-resource"), None);
        assert_eq!(
            parse_route("/.well-known/oauth-protected-resource/api/runtime/app/authentication_profile/p/extra"),
            None
        );
        assert_eq!(
            parse_route("/.well-known/oauth-protected-resource/api/runtime/app/profile/p"),
            None
        );
    }

    #[test]
    fn test_parse_route_rejects_invalid_ids() {
        for id in ["a:b", "..", "a%2Fb", &"a".repeat(65)] {
            let path = format!(
                "/.well-known/oauth-protected-resource/api/runtime/{id}/authentication_profile/p"
            );
            assert_eq!(parse_route(&path), None, "app id {id}");
            let path = format!(
                "/.well-known/oauth-protected-resource/api/runtime/app/authentication_profile/{id}"
            );
            assert_eq!(parse_route(&path), None, "profile id {id}");
        }
    }

    #[test]
    fn test_parse_route_rejects_empty_ids() {
        assert_eq!(
            parse_route(
                "/.well-known/oauth-protected-resource/api/runtime//authentication_profile/p"
            ),
            None
        );
    }

    #[test]
    fn test_id_validation() {
        assert!(is_valid_id(APP));
        assert!(is_valid_id("3f2a-44_x"));
        assert!(is_valid_id(&"a".repeat(64)));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("a:b"));
        assert!(!is_valid_id("../x"));
        assert!(!is_valid_id(&"a".repeat(65)));
    }

    #[test]
    fn test_find_document_returns_the_document() {
        let config = test_config();
        let document = find_document(
            &config,
            APP,
            &route("oauth-protected-resource", APP, "profile-1"),
        )
        .unwrap();
        assert_eq!(document["resource"], "https://app.betty.app");
    }

    #[test]
    fn test_find_document_other_application_is_not_found() {
        let config = test_config();
        let result = find_document(
            &config,
            "another-app",
            &route("oauth-protected-resource", APP, "profile-1"),
        );
        assert_eq!(result, Err(Failure::NotFound));
    }

    #[test]
    fn test_find_document_unknown_profile_is_not_found() {
        let config = test_config();
        let result = find_document(
            &config,
            APP,
            &route("oauth-protected-resource", APP, "profile-2"),
        );
        assert_eq!(result, Err(Failure::NotFound));
    }

    #[test]
    fn test_find_document_missing_sub_document_is_not_found() {
        let config = test_config();
        let result = find_document(
            &config,
            APP,
            &route("oauth-authorization-server", APP, "profile-1"),
        );
        assert_eq!(result, Err(Failure::NotFound));
    }
}
