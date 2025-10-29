use std::borrow::Cow;

use matchit::Match;
use matchit::Router;
use wasmcloud_component::http;
use wasmcloud_component::http::StatusCode;
use wasmcloud_component::wasi::logging::logging::log;
use wasmcloud_component::wasi::logging::logging::Level;

struct Component;

http::export!(Component);

pub mod bindings {
    wit_bindgen::generate!({ generate_all });
}

use crate::data_api::HelperContext;
use bindings::betty_blocks::data_api::data_api;

#[derive(Debug)]
enum Routes {
    Example,
    Data,
}

impl http::Server for Component {
    fn handle(
        request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>> {
        log(Level::Info, "", &format!("Hello to your logs from Rust"));

        let mut router = Router::new();
        router.insert("/example", Routes::Example).unwrap();
        router.insert("/data/{app_id}", Routes::Data).unwrap();
        router.insert("/data", Routes::Data).unwrap();

        let url: url::Url = request.uri().to_string().parse().unwrap();
        let path = request.uri().path();

        let out = match (request.method(), router.at(path)) {
            (
                &http::Method::GET,
                Ok(Match {
                    value: Routes::Data,
                    params,
                }),
            ) => {
                let app_id = params
                    .get("app_id")
                    .unwrap_or("478f55c1abf54460bc3d77d6787b1e82");
                let helper_context = HelperContext {
                    application_id: app_id.to_string(),
                    jwt: None,
                    action_id: "feae0f99dd4c423aae4d174fcd63ccdf".to_string(),
                    log_id: "123545".to_string(),
                    encrypted_configurations: None,
                };
                match data_api::request(&helper_context, "{allUser{results{id}}}", "{}") {
                    Ok(result) => result,
                    Err(result) => {
                        let mut response = http::Response::new(result);
                        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                        return Ok(response);
                    }
                }
            }
            (
                &http::Method::GET,
                Ok(Match {
                    value: Routes::Example,
                    ..
                }),
            ) => {
                let client = waki::Client::new();
                if let Ok(response) = client.get("https://example.com").send() {
                    let body = response.body().unwrap_or_default();
                    String::from_utf8_lossy(&body).to_string()
                } else {
                    String::from("request failed")
                }
            }
            (&http::Method::GET, _) => {
                let redirect = url
                    .query_pairs()
                    .find(|(key, _)| key == "redirect")
                    .map(|(_, url)| url)
                    .unwrap_or(Cow::from("https://example.com"));
                let client = waki::Client::new();
                if let Ok(response) = client.get(&redirect).send() {
                    let body = response.body().unwrap_or_default();
                    String::from_utf8_lossy(&body).to_string()
                } else {
                    String::from("request failed")
                }
            }
            _ => request
                .uri()
                .query()
                .map(|x| x.to_string())
                .unwrap_or(String::from("empty")),
        };

        Ok(http::Response::new(out))
    }
}
