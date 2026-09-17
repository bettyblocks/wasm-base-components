//! Announces that a wasm application deployment has gone live.
//!
//! Declared as the `service` of a Betty Blocks app WorkloadDeployment, so the
//! runtime runs it once per workload instance start (`wasi:cli/run`). It asks the
//! app component in its own workload which version it is serving via
//! `actions.health()` and POSTs that to builder-live-tracker, which compares it
//! with the version the compiler said it deployed and flips the IDE status.
//!
//! Exit code is the whole control protocol with the runtime:
//!
//! * exit 0 — done, do not run again. Used for success, for every non-retryable
//!   answer (any 4xx: a tracker without the route, a rejected token), and for
//!   missing configuration, which would otherwise burn `maxRestarts` in a loop.
//! * non-zero — retryable. The runtime restarts the service, bounded by
//!   `maxRestarts`. Used when the app is not answering `health()` yet, and when
//!   the tracker is unreachable or 5xx-ing after every attempt.

use std::{env, process::ExitCode, thread, time::Duration};

wit_bindgen::generate!({ generate_all });

use crate::betty_blocks_types::actions::actions;

/// Total POST attempts before giving up and letting the runtime restart us.
const MAX_ATTEMPTS: u32 = 5;

/// The only runtime this component is deployed into; sent so the tracker can key
/// its state the same way the compiler's events do.
const RUNTIME: &str = "wasm";

#[derive(Debug, PartialEq, Eq)]
struct Config {
    application_id: String,
    tracker_url: String,
    jaws_issuer: String,
    jaws_secret_key: String,
}

/// What to do after one POST attempt.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The tracker gave a final answer — stop, whether or not we liked it.
    Done,
    /// Transport or server-side trouble — worth another attempt.
    Retry,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// Reading through a lookup rather than straight from `env` keeps this
    /// testable: mutating the process environment races the test harness's own
    /// threads.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let required = |name: &str| {
            lookup(name).ok_or_else(|| format!("{name} must be set to announce a deployment"))
        };

        Ok(Self {
            application_id: required("APPLICATION_ID")?,
            tracker_url: required("LIVE_TRACKER_URL")?,
            jaws_issuer: lookup("JAWS_ISSUER").unwrap_or_else(|| "wasmcloud".to_string()),
            jaws_secret_key: required("ACTIONS_WASM_LIVE_TRACKER_SECRET")?,
        })
    }

    fn announce_url(&self) -> String {
        format!(
            "{}/internal/actions/{}/live",
            self.tracker_url.trim_end_matches('/'),
            self.application_id
        )
    }
}

fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            // Misconfiguration cannot fix itself, so do not let the runtime retry it.
            eprintln!("live-announcer config failed: {err}");
            return ExitCode::SUCCESS;
        }
    };

    let version = match actions::health() {
        Ok(version) => version,
        Err(err) => {
            // The app component may still be warming up — this one is worth a restart.
            eprintln!("live-announcer health check failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    announce(&config, &version)
}

fn announce(config: &Config, version: &str) -> ExitCode {
    let url = config.announce_url();

    let observed_at = rfc3339_utc(jaws_rs::jsonwebtoken::get_current_timestamp());

    let body = match serde_json::to_vec(&announce_payload(version, &observed_at)) {
        Ok(body) => body,
        Err(err) => {
            eprintln!("live-announcer payload encode failed: {err}");
            return ExitCode::SUCCESS;
        }
    };

    let client = waki::Client::new();

    for attempt in 1..=MAX_ATTEMPTS {
        // Signed per attempt: a Jaws token lives for a minute, which the backoff
        // plus a few connect timeouts can outrun. An expired token comes back as
        // a 401, which we would read as a final refusal and exit on.
        let jwt = match generate_jaws(config) {
            Ok(jwt) => jwt,
            Err(err) => {
                eprintln!("live-announcer jwt generation failed: {err}");
                return ExitCode::SUCCESS;
            }
        };

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {jwt}"))
            .connect_timeout(Duration::from_secs(5))
            .body(body.clone())
            .send();

        match response {
            Ok(response) => {
                let status_code = response.status_code();

                if outcome_for_status(status_code) == Outcome::Done {
                    if !is_success(status_code) {
                        eprintln!(
                            "live-announcer announce rejected, not retrying \
                             | status_code={status_code} | url={url}"
                        );
                    }

                    return ExitCode::SUCCESS;
                }

                eprintln!(
                    "live-announcer announce returned a retryable status \
                     | status_code={status_code} | url={url} | attempt={attempt}"
                );
            }
            Err(err) => {
                eprintln!(
                    "live-announcer announce request failed \
                     | error={err} | url={url} | attempt={attempt}"
                );
            }
        }

        if let Some(backoff) = backoff(attempt) {
            thread::sleep(backoff);
        }
    }

    // Out of attempts. Exit non-zero so the runtime restarts us; `maxRestarts`
    // bounds how long we keep knocking.
    ExitCode::FAILURE
}

/// `state` and `observed_at` generalise the announce from "this version is up" to
/// "this is what I saw, and when". The tracker orders writers by `observed_at`,
/// which is what stops a late announce from a replaced instance clobbering a
/// fresher observation. This component only ever reports `live` — a component
/// cannot report its own death; that is the host watcher's job.
fn announce_payload(version: &str, observed_at: &str) -> serde_json::Value {
    serde_json::json!({
        "runtime": RUNTIME,
        "state": "live",
        "version": version,
        "observed_at": observed_at,
    })
}

/// RFC 3339 in UTC, to the second — the shape `DateTime.from_iso8601/1` parses on
/// the tracker side. Built by hand because the component has no date library and
/// `wasi:clocks/wall-clock` gives us a bare Unix timestamp.
fn rfc3339_utc(unix_seconds: u64) -> String {
    let (days, seconds) = (unix_seconds / 86_400, unix_seconds % 86_400);
    let (hour, minute, second) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    let (year, month, day) = civil_from_days(days as i64);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's days-from-civil, inverted. Exact for every date we can see.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };

    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn is_success(status_code: u16) -> bool {
    (200..300).contains(&status_code)
}

/// 2xx means the tracker took it; 4xx means it will never take it (no such route,
/// bad token, malformed body). Both are final. Everything else — 5xx, and any
/// shape we did not expect — gets another go.
fn outcome_for_status(status_code: u16) -> Outcome {
    if is_success(status_code) || (400..500).contains(&status_code) {
        Outcome::Done
    } else {
        Outcome::Retry
    }
}

/// Exponential backoff between attempts. `None` after the final attempt, so we
/// hand control back to the runtime instead of sleeping for nothing.
fn backoff(attempt: u32) -> Option<Duration> {
    if attempt >= MAX_ATTEMPTS {
        None
    } else {
        Some(Duration::from_secs(1 << (attempt - 1)))
    }
}

fn generate_jaws(config: &Config) -> Result<String, String> {
    let issued_at = jaws_rs::jsonwebtoken::get_current_timestamp();

    let claims = jaws_rs::Claims::new(
        config.jaws_issuer.clone(),
        config.application_id.clone(),
        issued_at,
        uuid::Uuid::new_v4().to_string(),
    );

    jaws_rs::encode(&claims, &config.jaws_secret_key)
        .map_err(|e| format!("Failed to encode JAWS token: {e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            application_id: "11111111111111111111111111111111".to_string(),
            tracker_url: "http://builder-live-tracker:4000".to_string(),
            jaws_issuer: "wasmcloud".to_string(),
            jaws_secret_key: "secret".to_string(),
        }
    }

    #[test]
    fn when_building_the_announce_url_it_should_point_at_the_internal_route() {
        assert_eq!(
            config().announce_url(),
            "http://builder-live-tracker:4000/internal/actions/11111111111111111111111111111111/live"
        );
    }

    #[test]
    fn when_the_tracker_url_has_a_trailing_slash_it_should_not_double_up() {
        let config = Config {
            tracker_url: "http://builder-live-tracker:4000/".to_string(),
            ..config()
        };

        assert_eq!(
            config.announce_url(),
            "http://builder-live-tracker:4000/internal/actions/11111111111111111111111111111111/live"
        );
    }

    #[test]
    fn when_building_the_payload_it_should_carry_the_state_and_observation_time() {
        assert_eq!(
            announce_payload("v0.1.0-2026-09-15T10:04:02Z", "2026-09-15T10:04:07Z"),
            serde_json::json!({
                "runtime": "wasm",
                "state": "live",
                "version": "v0.1.0-2026-09-15T10:04:02Z",
                "observed_at": "2026-09-15T10:04:07Z"
            })
        );
    }

    #[test]
    fn when_formatting_a_timestamp_it_should_match_rfc3339_utc() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day, and the last second before one.
        assert_eq!(rfc3339_utc(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_709_164_799), "2024-02-28T23:59:59Z");
        assert_eq!(rfc3339_utc(1_789_469_047), "2026-09-15T10:44:07Z");
        // Well past any clock this will run against.
        assert_eq!(rfc3339_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn when_the_tracker_accepts_the_announce_it_should_be_done() {
        assert_eq!(outcome_for_status(204), Outcome::Done);
        assert_eq!(outcome_for_status(200), Outcome::Done);
    }

    #[test]
    fn when_the_tracker_refuses_the_announce_it_should_be_done() {
        for status_code in [400, 401, 403, 404, 422] {
            assert_eq!(outcome_for_status(status_code), Outcome::Done);
        }
    }

    #[test]
    fn when_the_tracker_fails_server_side_it_should_retry() {
        for status_code in [500, 502, 503, 504] {
            assert_eq!(outcome_for_status(status_code), Outcome::Retry);
        }
    }

    #[test]
    fn when_backing_off_it_should_double_and_stop_after_the_last_attempt() {
        assert_eq!(backoff(1), Some(Duration::from_secs(1)));
        assert_eq!(backoff(2), Some(Duration::from_secs(2)));
        assert_eq!(backoff(3), Some(Duration::from_secs(4)));
        assert_eq!(backoff(4), Some(Duration::from_secs(8)));
        assert_eq!(backoff(MAX_ATTEMPTS), None);
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn when_every_variable_is_set_it_should_parse_the_config() {
        let parsed = Config::from_lookup(env(&[
            ("APPLICATION_ID", "11111111111111111111111111111111"),
            ("LIVE_TRACKER_URL", "http://builder-live-tracker:4000"),
            ("JAWS_ISSUER", "wasmcloud"),
            ("ACTIONS_WASM_LIVE_TRACKER_SECRET", "secret"),
        ]));

        assert_eq!(parsed, Ok(config()));
    }

    #[test]
    fn when_the_issuer_is_absent_it_should_default_to_wasmcloud() {
        let parsed = Config::from_lookup(env(&[
            ("APPLICATION_ID", "11111111111111111111111111111111"),
            ("LIVE_TRACKER_URL", "http://builder-live-tracker:4000"),
            ("ACTIONS_WASM_LIVE_TRACKER_SECRET", "secret"),
        ]));

        assert_eq!(parsed, Ok(config()));
    }

    #[test]
    fn when_a_required_variable_is_missing_it_should_name_it() {
        for missing in [
            "APPLICATION_ID",
            "LIVE_TRACKER_URL",
            "ACTIONS_WASM_LIVE_TRACKER_SECRET",
        ] {
            let pairs: Vec<(&str, &str)> = [
                ("APPLICATION_ID", "11111111111111111111111111111111"),
                ("LIVE_TRACKER_URL", "http://builder-live-tracker:4000"),
                ("ACTIONS_WASM_LIVE_TRACKER_SECRET", "secret"),
            ]
            .into_iter()
            .filter(|(key, _)| *key != missing)
            .collect();

            let error = Config::from_lookup(env(&pairs)).expect_err("should be missing config");
            assert!(error.contains(missing), "{error} should name {missing}");
        }
    }

    #[test]
    fn when_generating_jaws_token_it_should_return_token() {
        let token = generate_jaws(&config()).expect("token should be generated");
        assert!(!token.is_empty());
    }
}
