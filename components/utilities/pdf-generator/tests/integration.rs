mod bindings {
    wasmtime_testing_helper::bindgen!({ path: "wit", world: "provider" });

    wasmtime_testing_helper::setup!(Provider);
}

use jaws_rs::Keys;
use wasmtime_testing_helper::http::{HttpHandler, hyper};

use crate::bindings::betty_blocks_utilities::data_api::data_api::HelperContext;

const APPLICATION_ID: &str = "06caae5da8234837a330c14a7350ed75";
const JAWS_SECRET: &str = "SUPER_SECRET";
const JAWS_DEFAULT_ISSUER: &str = "actions-wasm";
const HTML: &str = "<html><body>Hello PDF</body></html>";
const PDF_BYTES: &[u8] = b"%PDF-1.7 fake pdf bytes";

fn helper_context() -> HelperContext {
    HelperContext {
        application_id: String::from(APPLICATION_ID),
        action_id: String::new(),
        log_id: String::new(),
        encrypted_configurations: None,
        jwt: None,
    }
}

fn pdf_generator_handler(status: u16, response_body: &'static [u8]) -> HttpHandler {
    Box::new(move |req, _conf| {
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let body_bytes = body.await?;

            assert_eq!(parts.method, hyper::Method::POST);
            assert_eq!(parts.uri.path(), format!("/html-to-pdf/{APPLICATION_ID}"));
            assert_eq!(
                parts.headers.get("content-type").unwrap().to_str().unwrap(),
                "text/html"
            );
            assert!(
                jaws_rs::decode_and_validate_jwt(
                    parts
                        .headers
                        .get("authorization")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .strip_prefix("Bearer ")
                        .unwrap(),
                    &Keys::from([(JAWS_DEFAULT_ISSUER.to_string(), JAWS_SECRET.to_string())])
                )
                .is_ok_and(|claims| claims.application_id == APPLICATION_ID)
            );
            assert_eq!(&body_bytes[..], HTML.as_bytes());

            let mut response = hyper::Response::new(hyper::body::Bytes::from_static(response_body));
            *response.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
            Ok(response)
        })
    })
}

fn instantiate_component(
    handler: Option<HttpHandler>,
    envs: &[(&str, &str)],
) -> wasmtime_testing_helper::InstantiatedComponent<bindings::Provider> {
    let mut harness = bindings::harness();
    harness
        .wasi_context_builder_mut()
        .envs(envs)
        .inherit_stdio();

    if let Some(handler) = handler {
        harness.mock_http_handler(handler);
    }

    bindings::instantiate(harness)
}

#[test]
fn pdf_generator_component_should_return_pdf_bytes_on_success() {
    let mut component = instantiate_component(
        Some(pdf_generator_handler(200, PDF_BYTES)),
        &[
            ("JAWS_SECRET_KEY", JAWS_SECRET),
            ("PDF_GENERATOR_URL", "http://pdf-generator:4000"),
        ],
    );

    let result = component
        .component
        .betty_blocks_utilities_pdf_generator_pdf_generator()
        .call_generate(&mut component.store, &helper_context(), HTML)
        .unwrap();

    assert_eq!(result.unwrap(), PDF_BYTES.to_vec());
}

#[test]
fn pdf_generator_component_should_return_error_on_non_2xx_status() {
    let mut component = instantiate_component(
        Some(pdf_generator_handler(500, b"kaboom")),
        &[
            ("JAWS_SECRET_KEY", JAWS_SECRET),
            ("PDF_GENERATOR_URL", "http://pdf-generator:4000"),
        ],
    );

    let result = component
        .component
        .betty_blocks_utilities_pdf_generator_pdf_generator()
        .call_generate(&mut component.store, &helper_context(), HTML)
        .unwrap();

    assert!(result.is_err_and(|err| err.contains("500") && err.contains("kaboom")));
}

#[test]
fn pdf_generator_component_should_return_config_error_when_jaws_secret_is_missing() {
    // No JAWS_SECRET_KEY and no http handler: the config check happens before any request is sent.
    let mut component = instantiate_component(None, &[]);

    let result = component
        .component
        .betty_blocks_utilities_pdf_generator_pdf_generator()
        .call_generate(&mut component.store, &helper_context(), HTML)
        .unwrap();

    assert!(result.is_err_and(|err| err.contains("JAWS_SECRET_KEY")));
}
