pub mod upload;

pub mod bindings {
    wit_bindgen::generate!({
        generate_all,
    });
}

use bindings::{
    betty_blocks::data_api::data_api::HelperContext,
    betty_blocks::types::types::{Model, Property},
    exports::betty_blocks::file::upload_file::{Guest as UploadFileGuest, UploadResult},
};

use crate::upload::upload_bytes_internal;

struct Component;

impl UploadFileGuest for Component {
    fn upload(
        helper_context: HelperContext,
        model: Model,
        property: Property,
        file_bytes: Vec<u8>,
        full_filename: String,
    ) -> Result<UploadResult, String> {
        wstd::runtime::block_on(upload_bytes_internal(
            helper_context,
            model,
            property,
            file_bytes,
            full_filename,
        ))
        .map_err(|e| e.to_string())
    }
}

bindings::export!(Component with_types_in bindings);
