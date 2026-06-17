pub mod upload;

pub mod bindings {
    wit_bindgen::generate!({
        generate_all,
    });
}

use bindings::{
    betty_blocks_utilities::data_api::data_api::HelperContext,
    exports::betty_blocks_utilities::upload_file::upload_file::{Guest as UploadFileGuest, Input, UploadResult},
};

use crate::upload::upload_bytes_internal;

struct Component;

impl UploadFileGuest for Component {
    fn upload(
        helper_context: HelperContext,
        Input {
            model,
            property,
            file_bytes,
            full_filename,
        }: Input,
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
