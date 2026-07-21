use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::context::data_structs::{DeleteInput, MutationInput};
use crate::context::format_mutation::{
    generate_delete_many_inputs, generate_upsert_many_inputs, send_delete_many_mutations,
    send_upsert_many_mutations,
};
use crate::exports::betty_blocks_types::data_api::data_api::{self, GuestDataApi, JsonString};
use crate::{Config, inner_request};

mod data_structs;
mod format_mutation;
mod process_request;
mod replace_ids;

type ModelName = String;
type InternalId = isize;
type RealId = i32;

pub struct DataAPIContext {
    application_id: String,
    #[expect(dead_code, reason = "Will be used for logging in the future")]
    action_id: String,
    /// jwt of the customer (so not the jaws jwt used for authenticating the server to server communication)
    jwt: Option<String>,
    capture_data: Arc<Mutex<CaptureData>>,
}

#[derive(Debug, Default)]
pub struct CaptureData {
    model_names_of_local_ids: Vec<ModelName>,
    reserve_id_count_per_model: HashMap<ModelName, u32>,
    reserved_ids: Vec<RealId>,
    capture_stack: Vec<MassMutateEntries>,
}

#[derive(Default, Debug)]
pub struct MassMutateEntries {
    create: Vec<MassMutateEntry<MutationInput>>,
    update: Vec<MassMutateEntry<MutationInput>>,
    delete: Vec<MassMutateEntry<DeleteInput>>,
}

impl MassMutateEntries {
    fn as_pending_mutations(&self) -> [Vec<data_api::PendingMutation>; 3] {
        [
            self.create
                .iter()
                .map(|entry| entry.as_pending_mutation("create"))
                .collect(),
            self.update
                .iter()
                .map(|entry| entry.as_pending_mutation("update"))
                .collect(),
            self.delete
                .iter()
                .map(|entry| entry.as_pending_mutation("delete"))
                .collect(),
        ]
    }
}

#[derive(Debug)]
pub struct MassMutateEntry<T: serde::Serialize> {
    model_name: String,
    variables: T,
}

pub trait RequestRaw {
    fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String>;
}

impl<T: GuestDataApi> RequestRaw for T {
    fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        GuestDataApi::request_raw(self, query, variables)
    }
}

impl GuestDataApi for DataAPIContext {
    fn new(application_id: String, action_id: String, jwt: Option<String>) -> Self {
        DataAPIContext {
            application_id,
            action_id,
            jwt,
            capture_data: Default::default(),
        }
    }

    fn apply_capture(&self) -> Result<(), String> {
        let mut capture_data = self
            .capture_data
            .lock()
            .map_err(|_| String::from("capture stack lock poisoned"))?;
        if let Some(MassMutateEntries {
            create,
            update,
            delete,
        }) = capture_data.capture_stack.pop()
        {
            let reserved_ids = capture_data.reserve_ids(self)?;

            send_upsert_many_mutations(
                generate_upsert_many_inputs(create, update, &reserved_ids)?,
                self,
            )?;

            send_delete_many_mutations(generate_delete_many_inputs(delete, &reserved_ids)?, self)?;
        }

        Ok(())
    }

    fn discard_capture(&self) -> Result<String, String> {
        if self
            .capture_data
            .lock()
            .expect("lock is poisoned")
            .capture_stack
            .pop()
            .is_some()
        {
            return Ok(String::from("deleted most recent capture stack entry"));
        }

        Ok(String::from("nothing to do"))
    }

    fn start_capture(&self) -> Result<(), String> {
        self.capture_data
            .lock()
            .map_err(|_| String::from("capture stack lock poisoned"))?
            .capture_stack
            .push(Default::default());
        Ok(())
    }

    fn pending_capture(&self) -> Result<Vec<Vec<data_api::PendingMutation>>, String> {
        if let Some(entries) = self
            .capture_data
            .lock()
            .expect("lock is poisoned")
            .capture_stack
            .last()
        {
            return Ok(entries.as_pending_mutations().to_vec());
        }

        Ok(Vec::new())
    }

    fn request(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        if let Ok(mut capture_data) = self.capture_data.lock()
            && !capture_data.capture_stack.is_empty()
        {
            process_request::extract_mutation_data(self, &mut capture_data, query, variables)
        } else {
            RequestRaw::request_raw(self, query, variables)
        }
    }

    fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        let config = match Config::from_env() {
            Ok(config) => config,
            Err(e) => return Err(format!("Configuration error: {e:#}")),
        };

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Err(format!("failed to create tokio runtime: {e}")),
        };

        runtime
            .block_on(inner_request(
                config,
                &self.application_id,
                self.jwt.clone(),
                query,
                variables,
            ))
            .map_err(|e| format!("{e:#}"))
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::*;

    impl Default for DataAPIContext {
        fn default() -> Self {
            DataAPIContext::new(Default::default(), Default::default(), Default::default())
        }
    }

    pub struct RequestRawMock {
        should_expect: bool,
        expected_requests: RefCell<VecDeque<(String, JsonString)>>,
        responses: RefCell<VecDeque<Result<JsonString, String>>>,
    }

    impl Default for RequestRawMock {
        fn default() -> Self {
            Self::without_expect()
        }
    }

    impl RequestRawMock {
        pub fn without_expect() -> Self {
            Self {
                should_expect: false,
                expected_requests: Default::default(),
                responses: Default::default(),
            }
        }

        pub fn with_expect(
            expected_requests: VecDeque<(String, JsonString)>,
            responses: VecDeque<Result<JsonString, String>>,
        ) -> Self {
            Self {
                should_expect: true,
                expected_requests: RefCell::new(expected_requests),
                responses: RefCell::new(responses),
            }
        }
    }

    impl RequestRaw for RequestRawMock {
        fn request_raw(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
            if self.should_expect {
                if let Some((expected_query, expected_variables)) =
                    self.expected_requests.borrow_mut().pop_front()
                {
                    if expected_query != query || expected_variables != variables {
                        panic!(
                            "Unexpected request_raw call.\nExpected:\n{expected_query}\nwith variables:\n{expected_variables}\nGot:\n{query}\nwith variables:\n{variables}"
                        );
                    }
                } else {
                    panic!(
                        "Unexpected request_raw call.\nExpected nothing\nGot:\n{query}\nwith variables:\n{variables}"
                    );
                }
            }

            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| String::from("Default mock response"))
                .flatten()
        }
    }
}
