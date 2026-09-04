#![allow(clippy::unwrap_used)]

//! Characterizes the `ATLAS_ANCHOR_INTERVAL` divergence pinned by design
//! D3.3/R4: `ServerConfig::from_env` (via `read_anchor_interval`) refuses a
//! value below 2, while `AppState::new`/`for_test`'s own read clamps the same
//! value to a floor of 2 instead of rejecting it. PR2 collapses this to one
//! rule (the strict one, design D3.3); this test names the contradiction
//! explicitly so it is resolved deliberately, not silently.

use atlas_core::config::EnvSource;
use atlas_server::config::read_anchor_interval;

struct OneVar(&'static str, &'static str);

impl EnvSource for OneVar {
    fn get(&self, key: &str) -> Option<String> {
        (key == self.0).then(|| self.1.to_string())
    }
}

#[test]
fn strict_loader_rejects_anchor_interval_below_2() {
    let source = OneVar("ATLAS_ANCHOR_INTERVAL", "1");

    let result = read_anchor_interval(&source);

    assert!(
        result.is_err(),
        "ServerConfig's strict rule must reject ATLAS_ANCHOR_INTERVAL=1"
    );
    assert!(
        result.unwrap_err().contains("ATLAS_ANCHOR_INTERVAL"),
        "the refusal must name the variable"
    );
}

#[test]
fn strict_loader_accepts_anchor_interval_at_the_floor() {
    let source = OneVar("ATLAS_ANCHOR_INTERVAL", "2");

    assert_eq!(read_anchor_interval(&source), Ok(2));
}

#[test]
fn app_state_reader_clamps_the_same_value_instead_of_rejecting_it() {
    // Mirrors `AppState::new`/`for_test`'s own read
    // (`read_env::<u32>(source, "ATLAS_ANCHOR_INTERVAL", 50).max(2)`): the
    // same out-of-range value that `read_anchor_interval` rejects is
    // silently floored to 2 on this path today. Design D3.3 deletes this
    // clamp on the production path in PR2 and keeps only the strict rule;
    // `AppState::for_test`'s clamp is kept unchanged by design because it
    // constructs no config object.
    let source = OneVar("ATLAS_ANCHOR_INTERVAL", "1");
    let raw = atlas_server::config::read_env::<u32>(&source, "ATLAS_ANCHOR_INTERVAL", 50);
    let clamped = raw.max(2);

    assert_eq!(
        clamped, 2,
        "AppState's reader floors an out-of-range value to 2 rather than rejecting it"
    );
}
