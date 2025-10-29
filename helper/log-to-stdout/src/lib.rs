use crate::exports::wasi::logging::logging::{self, Guest};
// use tracing::dispatcher::{self, Dispatch};
// use tracing_subscriber::Registry;
// use tracing_subscriber::layer::SubscriberExt;

wit_bindgen::generate!({ generate_all });

struct Logger;

impl Guest for Logger {
    fn log(level: logging::Level, _context: String, message: String) -> () {
        // let subscriber = Registry::default().with(tracing_logfmt::layer());
        // dispatcher::set_global_default(Dispatch::new(subscriber)).ok();
        tracing_subscriber::fmt().try_init().ok();

        inner_log(level, &message);
    }
}

fn inner_log(severity: logging::Level, data: &str) {
    match severity {
        logging::Level::Trace => tracing::trace!(data),
        logging::Level::Debug => tracing::debug!(data),
        logging::Level::Info => tracing::info!(data),
        logging::Level::Warn => tracing::warn!(data),
        logging::Level::Error | logging::Level::Critical => tracing::error!(data),
    }
}

export! {Logger}
