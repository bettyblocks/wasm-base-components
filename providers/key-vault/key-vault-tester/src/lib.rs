mod bindings {
    use crate::KeyFetcher;
    use wasmcloud_component::http;

    wit_bindgen::generate!({
        with: {
            "wasi:clocks/monotonic-clock@0.2.2": wasmcloud_component::wasi::clocks::monotonic_clock,
            "wasi:http/incoming-handler@0.2.2": generate,
            "wasi:http/types@0.2.2": wasmcloud_component::wasi::http::types,
            "wasi:io/error@0.2.2": wasmcloud_component::wasi::io::error,
            "wasi:io/poll@0.2.2": wasmcloud_component::wasi::io::poll,
            "wasi:io/streams@0.2.2": wasmcloud_component::wasi::io::streams,
            "betty-blocks:key-vault/key-vault": generate
        }
    });

    http::export!(KeyFetcher);
}

use bindings::betty_blocks::key_vault::key_vault;
use wasmcloud_component::http;

struct KeyFetcher;

const EMPTY_MESSAGE: &str = "empty :(";

impl http::Server for KeyFetcher {
    fn handle(
        request: http::IncomingRequest,
    ) -> http::Result<http::Response<impl http::OutgoingBody>, http::ErrorCode> {
        let key = request.uri().path().replace("/", "");
        let outkey = key_vault::get_secret(&key).unwrap_or(String::from(EMPTY_MESSAGE));
        Ok(http::Response::new(outkey))
    }
}
