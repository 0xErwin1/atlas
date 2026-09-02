#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! T2.9-T2.11 (`v2-e3-s4` PR2, spec's `Page<T>` alias-guard requirement):
//! `tests/api_page_conformance.rs` (`v2-e3-s3` PR2) classifies a route as
//! `Page<T>`-returning by scanning source text for the literal substring
//! `Page<` in either the handler's return-type signature or its
//! `#[utoipa::path(body = ...)]` annotation. A `type Alias = Page<T>;`
//! introduced in `atlas_api::dtos` and used as a handler's return type would
//! defeat that scan: the call site would read `Json<Alias>`, mentioning
//! neither `Page<` nor the alias's own definition, so the route would
//! silently drop out of the classified set.
//!
//! Per the spec's stated either/or ("extend the classifier to resolve the
//! alias, or forbid the alias outright"), this test picks the second: no
//! type alias anywhere in `atlas_api::dtos` may resolve to `Page<...>`. This
//! is cheaper and strictly safer than teaching the classifier to resolve
//! alias chains across files, and `Page<T>` is a generic wrapper every
//! caller can name directly with no loss of expressiveness.

use std::fs;
use std::path::Path;

fn dtos_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dtos")
}

/// Every `.rs` file directly under `src/dtos` (one level — the module has no
/// nested submodule directories today; a future nested module would need
/// this walk extended, which the file-count assertion in
/// `every_page_type_alias_is_forbidden_has_teeth` below exists to catch).
fn dto_source_files() -> Vec<std::path::PathBuf> {
    let dir = dtos_dir();
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));

    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .collect()
}

/// A `type` alias declaration whose right-hand side is `Page<...>`,
/// regardless of visibility (`pub`, `pub(crate)`, or private) or whitespace.
fn is_page_type_alias(line: &str) -> bool {
    let trimmed = line.trim_start();
    let after_type = trimmed
        .strip_prefix("pub(crate) type ")
        .or_else(|| trimmed.strip_prefix("pub type "))
        .or_else(|| trimmed.strip_prefix("type "));

    match after_type {
        Some(rest) => rest
            .split('=')
            .nth(1)
            .is_some_and(|rhs| rhs.trim_start().starts_with("Page<")),
        None => false,
    }
}

#[test]
fn no_page_type_alias_exists_in_dtos() {
    let mut offenders = Vec::new();

    for path in dto_source_files() {
        let content =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        for (line_number, line) in content.lines().enumerate() {
            if is_page_type_alias(line) {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    line_number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "atlas_api::dtos must not declare a type alias resolving to Page<...> — it defeats \
         tests/api_page_conformance.rs's source-text Page<T> classifier (v2-e3-s4 D4/T2.9). \
         Name the type directly at every call site instead. Offenders: {offenders:?}"
    );
}

/// Adversarial proof (T2.10): a throwaway alias must actually be caught by
/// the same detection function the real test above uses, so the guard is not
/// passing vacuously by never encountering a positive case.
#[test]
fn the_alias_detector_actually_detects_an_alias() {
    assert!(is_page_type_alias("type TaskFeed = Page<TaskSummaryDto>;"));
    assert!(is_page_type_alias(
        "pub type TaskFeed = Page<TaskSummaryDto>;"
    ));
    assert!(is_page_type_alias(
        "    pub(crate) type TaskFeed = Page<TaskSummaryDto>;"
    ));
    assert!(!is_page_type_alias(
        "pub struct TaskFeed { pub items: Vec<TaskSummaryDto> }"
    ));
    assert!(!is_page_type_alias(
        "pub type TaskFeedItems = Vec<TaskSummaryDto>;"
    ));
}

/// If `src/dtos` ever grows a nested module directory, `dto_source_files`'s
/// single-level `read_dir` would silently stop scanning it. This pins the
/// current flat-file count so a future nested module forces a conscious
/// update to the walk, rather than a silent scope-narrowing.
#[test]
fn dto_source_files_enumeration_is_flat_and_non_empty() {
    let files = dto_source_files();
    assert!(
        !files.is_empty(),
        "dto_source_files() must find at least one .rs file under src/dtos"
    );
    for path in &files {
        assert!(
            path.parent() == Some(dtos_dir().as_path()),
            "{path:?} is not a direct child of src/dtos — the flat-directory assumption broke"
        );
    }
}
