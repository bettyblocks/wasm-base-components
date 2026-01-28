use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::exports::betty_blocks::data_api::data_api::{
     GuestDataApi,  JsonString,
};
use crate::{inner_request, Config};

pub struct DataAPIContext {
    application_id: String,
    action_id: String,
    /// jwt of the customer (so not the jaws jwt used for authenticating the server to server communication)
    jwt: Option<String>,
    mass_mutate_entries: Arc<Mutex<HashMap<String, MassMutateEntries>>>,
    capture_stack: Arc<Mutex<Vec<String>>>,
}

#[derive(Default)]
pub struct MassMutateEntries {
    create: VecDeque<MassMutateEntry>,
    update: VecDeque<MassMutateEntry>,
    delete: VecDeque<MassMutateEntry>,
}

// replace with real serde_json::Value when implementing
#[allow(non_camel_case_types)]
type serde_json_Value = String;

pub struct MassMutateEntry {
    model_name: String,
    variables: serde_json_Value,
}

impl GuestDataApi for DataAPIContext {
    fn new(application_id: String, action_id: String, jwt: Option<String>) -> Self {
        DataAPIContext {
            application_id,
            action_id,
            jwt,
            mass_mutate_entries: Default::default(),
            capture_stack: Default::default(),
        }
    }

    fn apply_capture(&self) -> Result<String, String> {
        todo!("construct the different mutations and apply them")
    }

    fn discard_capture(&self) -> Result<String, String> {
        if let Some(capture_id) = self.capture_stack.lock().expect("lock is poisoned").pop() {
            match self
                .mass_mutate_entries
                .lock()
                .expect("lock is poisoned")
                .remove(&capture_id)
            {
                Some(_) => return Ok(format!("deleted capture entry with id {capture_id}")),
                None => {
                    return Ok(format!(
                        "capture entry with id {capture_id} was already deleted"
                    ))
                }
            }
        }

        Ok(String::from("nothing to do"))
    }

    fn start_capture(&self) -> Result<String, String> {
        let random_capture_id = String::from("hallo1234");
        {
            self.mass_mutate_entries
                .lock()
                .expect("lock is poisoned")
                .insert(random_capture_id.clone(), MassMutateEntries::default());
        }

        {
            self.capture_stack
                .lock()
                .expect("lock is poisoned")
                .push(random_capture_id.clone());
        }

        Ok(random_capture_id)
    }

    fn request(&self, query: String, variables: JsonString) -> Result<JsonString, String> {
        if let Ok(capture_stack) = self.capture_stack.lock() {
            if capture_stack.len() > 0 {
                todo!("parse gql and put to correct capture stack if it is a single create,update,delete. If it is a many let it continue")
            }
        }

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
