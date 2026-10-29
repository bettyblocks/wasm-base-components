use serde_json::Value;
use wstd::http::{Body, Method, Request, Response, StatusCode};

wit_bindgen::generate!({
    world: "well-known",
    generate_all,
});

mod config;

const MAX_ID_LENGTH: usize = 64;

/// The kind of document a path serves.
#[derive(Debug, Clone, Copy)]
enum Document {
    ProtectedResource,
    AuthorizationServer,
}

#[derive(Debug, PartialEq)]
enum Route {
    /// The metadata of one mcp server, `resource` of the document is the url of that server.
    ProtectedResource { mcp_id: String },
    /// The metadata of the authorization server of one authentication profile.
    AuthorizationServer {
        app_uuid: String,
        profile_id: String,
    },
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
    match route {
        Route::ProtectedResource { mcp_id } => config.protected_resources.get(mcp_id),
        Route::AuthorizationServer {
            app_uuid,
            profile_id,
        } if app_uuid == application_id => config.authorization_servers.get(profile_id),
        Route::AuthorizationServer { .. } => None,
    }
    .ok_or(Failure::NotFound)
}

/// Matches
/// - `/.well-known/oauth-protected-resource/api/mcp/{mcp_id}`, the url of the mcp server appended
///   to the well-known prefix (RFC 9728 section 3.1)
/// - `/.well-known/oauth-authorization-server/api/runtime/{app_uuid}/authentication_profile/{profile_id}`
///
/// Anything else, including an app or profile id with characters outside `[A-Za-z0-9_-]`, is no
/// route. The mcp id is not validated: it is the path of the folder of the mcp server and is only
/// looked up in the config of the app.
fn parse_route(path: &str) -> Option<Route> {
    let mut router = matchit::Router::new();
    router
        .insert(
            "/.well-known/oauth-protected-resource/api/mcp/{mcp_id}",
            Document::ProtectedResource,
        )
        .ok()?;
    router
        .insert(
            "/.well-known/oauth-authorization-server/api/runtime/{app_uuid}/authentication_profile/{profile_id}",
            Document::AuthorizationServer,
        )
        .ok()?;

    let matched = router.at(path).ok()?;
    match matched.value {
        Document::ProtectedResource => Some(Route::ProtectedResource {
            mcp_id: matched.params.get("mcp_id")?.to_string(),
        }),
        Document::AuthorizationServer => {
            let app_uuid = matched.params.get("app_uuid")?;
            let profile_id = matched.params.get("profile_id")?;
            if !is_valid_id(app_uuid) || !is_valid_id(profile_id) {
                return None;
            }

            Some(Route::AuthorizationServer {
                app_uuid: app_uuid.to_string(),
                profile_id: profile_id.to_string(),
            })
        }
    }
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
                    "oauth-protected-resource": {
                        "server-1": { "resource": "https://app.betty.app/api/mcp/server-1" },
                        "server-2": { "resource": "https://app.betty.app/api/mcp/server-2" }
                    },
                    "oauth-authorization-server": {
                        "profile-1": { "issuer": "https://app.betty.app/profile-1" }
                    }
                }
            }"#,
        )
        .unwrap()
    }

    fn protected_resource(mcp_id: &str) -> Route {
        Route::ProtectedResource {
            mcp_id: mcp_id.to_string(),
        }
    }

    fn authorization_server(app_uuid: &str, profile_id: &str) -> Route {
        Route::AuthorizationServer {
            app_uuid: app_uuid.to_string(),
            profile_id: profile_id.to_string(),
        }
    }

    #[test]
    fn test_parse_route_protected_resource() {
        assert_eq!(
            parse_route("/.well-known/oauth-protected-resource/api/mcp/server-1"),
            Some(protected_resource("server-1"))
        );
    }

    #[test]
    fn test_parse_route_authorization_server() {
        let path = format!(
            "/.well-known/oauth-authorization-server/api/runtime/{APP}/authentication_profile/profile-1"
        );
        assert_eq!(
            parse_route(&path),
            Some(authorization_server(APP, "profile-1"))
        );
    }

    #[test]
    fn test_parse_route_rejects_other_paths() {
        assert_eq!(parse_route("/"), None);
        assert_eq!(parse_route("/api/mcp/server-1"), None);
        assert_eq!(parse_route("/.well-known/oauth-protected-resource"), None);
        assert_eq!(
            parse_route("/.well-known/oauth-protected-resource/api/mcp/"),
            None
        );
        assert_eq!(
            parse_route("/.well-known/oauth-protected-resource/api/mcp/server-1/extra"),
            None
        );
        assert_eq!(
            parse_route("/.well-known/openid-configuration/api/mcp/server-1"),
            None
        );
    }

    #[test]
    fn test_parse_route_authorization_server_rejects_other_paths() {
        for path in [
            // the protected resource is described per mcp server, not per profile
            format!("/.well-known/oauth-protected-resource/api/runtime/{APP}/authentication_profile/profile-1"),
            format!("/.well-known/oauth-authorization-server/api/runtime/{APP}/authentication_profile/p/extra"),
            format!("/.well-known/oauth-authorization-server/api/runtime/{APP}/profile/p"),
            "/.well-known/oauth-authorization-server/api/runtime//authentication_profile/p"
                .to_string(),
        ] {
            assert_eq!(parse_route(&path), None, "{path}");
        }
    }

    #[test]
    fn test_parse_route_rejects_invalid_ids() {
        for id in ["a:b", "..", "a%2Fb", &"a".repeat(65)] {
            let path = format!(
                "/.well-known/oauth-authorization-server/api/runtime/{id}/authentication_profile/p"
            );
            assert_eq!(parse_route(&path), None, "app id {id}");
            let path = format!(
                "/.well-known/oauth-authorization-server/api/runtime/app/authentication_profile/{id}"
            );
            assert_eq!(parse_route(&path), None, "profile id {id}");
        }
    }

    #[test]
    fn test_parse_route_does_not_validate_the_mcp_id() {
        // the name of an mcp server folder can be long and is not limited to an id alphabet
        let long = "participant-context-lookup-and-portfolio-overview-".repeat(8);
        for id in ["a.b", "a b", "a:b", long.as_str()] {
            let path = format!("/.well-known/oauth-protected-resource/api/mcp/{id}");
            assert_eq!(parse_route(&path), Some(protected_resource(id)), "{id}");
        }
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
    fn test_find_document_protected_resource_per_mcp_server() {
        let config = test_config();
        for mcp_id in ["server-1", "server-2"] {
            let document = find_document(&config, APP, &protected_resource(mcp_id)).unwrap();
            assert_eq!(
                document["resource"],
                format!("https://app.betty.app/api/mcp/{mcp_id}")
            );
        }
    }

    #[test]
    fn test_find_document_unknown_mcp_server_is_not_found() {
        let config = test_config();
        let result = find_document(&config, APP, &protected_resource("server-3"));
        assert_eq!(result, Err(Failure::NotFound));
    }

    #[test]
    fn test_find_document_authorization_server() {
        let config = test_config();
        let document =
            find_document(&config, APP, &authorization_server(APP, "profile-1")).unwrap();
        assert_eq!(document["issuer"], "https://app.betty.app/profile-1");
    }

    #[test]
    fn test_find_document_other_application_is_not_found() {
        let config = test_config();
        let result = find_document(
            &config,
            "another-app",
            &authorization_server(APP, "profile-1"),
        );
        assert_eq!(result, Err(Failure::NotFound));
    }

    #[test]
    fn test_find_document_unknown_profile_is_not_found() {
        let config = test_config();
        let result = find_document(&config, APP, &authorization_server(APP, "profile-2"));
        assert_eq!(result, Err(Failure::NotFound));
    }

    #[test]
    fn test_find_document_missing_kind_is_not_found() {
        let config = parse_config(r#"{"well-known": {}}"#).unwrap();
        assert_eq!(
            find_document(&config, APP, &protected_resource("server-1")),
            Err(Failure::NotFound)
        );
        assert_eq!(
            find_document(&config, APP, &authorization_server(APP, "profile-1")),
            Err(Failure::NotFound)
        );
    }
}
