#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! T2.1-T2.4 (`v2-e3-s3` PR2): RFC 3339 date field conformance.
//!
//! Every field in `crates/atlas_api/src/dtos/*` whose name matches the
//! `*_at`/`*_date` convention (or is otherwise documented as a timestamp)
//! MUST be declared `chrono::DateTime<chrono::Utc>` (bare or wrapped in
//! `Option<..>`), never a raw `String`.
//!
//! Mechanism (source-level, data-driven, not a curated field list): Rust has
//! no runtime struct-field reflection, and pulling in `syn` just to parse
//! field types was rejected by design (design doc §11: "a syn-free approach
//! is unnecessary"). This test instead re-reads every `.rs` file directly
//! under `dtos/` on each run and greps the actual source text for `pub
//! <name>: <type>,` field declarations inside `struct` bodies, AND for
//! `<name>: <type>,` field declarations (no `pub`, since Rust does not allow
//! one there and serde serializes them unconditionally) inside a struct-like
//! enum variant's own body. Because the directory itself is walked
//! (`fs::read_dir`, not a fixed list of module names) and every field in
//! every struct and every enum variant is inspected (not a fixed list of
//! field names), a brand-new DTO file or a brand-new field is covered
//! automatically the next time this test runs — there is nowhere to
//! register it and nowhere it can be silently skipped. `#[cfg(test)]`
//! modules are stripped from each file's text before scanning, so no
//! test-only fixture struct ever contributes a false positive or false
//! negative to the DTO field set.
//!
//! This is a structural check, not a live request: it never spins up a
//! server or a database, matching this test's placement in `atlas_api`
//! (a pure types crate with no HTTP framework dependency).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

const CONFORMANT_TYPE: &str = "chrono::DateTime<chrono::Utc>";
const CONFORMANT_TYPE_OPTION: &str = "Option<chrono::DateTime<chrono::Utc>>";

/// A field discovered by the source scan.
#[derive(Debug, Clone)]
struct DiscoveredField {
    file: String,
    struct_name: String,
    field_name: String,
    type_text: String,
    line: usize,
}

impl DiscoveredField {
    fn is_date_named(&self) -> bool {
        self.field_name.ends_with("_at") || self.field_name.ends_with("_date")
    }

    fn is_conformant_type(&self) -> bool {
        self.type_text == CONFORMANT_TYPE || self.type_text == CONFORMANT_TYPE_OPTION
    }
}

/// Fields whose name matches the date-shaped naming convention but are
/// deliberately not `chrono::DateTime<chrono::Utc>`, with the reason each
/// exists. Scoped to (file, struct, field) so an exemption never widens to
/// cover a same-named field in a different struct.
///
/// This list is self-checking both ways: `exemptions_are_still_accurate`
/// below fails if a listed (file, struct, field) triple no longer exists in
/// the scanned source, or if its live type text no longer matches the
/// reason recorded here — an exemption cannot silently keep excluding a
/// field whose shape has since changed.
const EXEMPTIONS: &[(&str, &str, &str, &str)] = &[(
    "boards_tasks.rs",
    "UpdateTaskRequest",
    "due_date",
    "PATCH field-presence wrapper: `present_value` distinguishes \
     absent/explicit-null/present so a client can clear the field, which \
     needs `Option<serde_json::Value>` — the same treatment as its sibling \
     `priority`/`estimate` fields on the same struct, not a raw String \
     standing in for a date.",
)];

fn dtos_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/dtos")
}

/// Removes every `#[cfg(test)] mod ... { ... }` block from `source` by
/// brace-depth tracking, so test-only fixture structs never enter the scan.
///
/// A file can contain more than one such block interleaved with production
/// structs (this codebase's convention puts a `#[cfg(test)] mod
/// foo_tests { ... }` directly after the code it exercises, not once at the
/// bottom of the file) — truncating at the first occurrence would silently
/// drop every production struct declared after it, which is worse than not
/// stripping at all. This walks the whole file once, counting braces to find
/// each test module's exact end.
fn strip_test_modules(source: &str) -> Vec<(usize, &str)> {
    let mut kept = Vec::new();
    let mut skip_from_depth: Option<i32> = None;
    let mut depth = 0i32;
    let mut cfg_test_pending = false;

    for (idx, line) in source.lines().enumerate() {
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;

        if let Some(base) = skip_from_depth {
            depth += opens - closes;
            if depth <= base {
                skip_from_depth = None;
            }
            continue;
        }

        let trimmed = line.trim();

        if trimmed == "#[cfg(test)]" {
            cfg_test_pending = true;
            depth += opens - closes;
            continue;
        }

        if cfg_test_pending {
            cfg_test_pending = false;
            if trimmed.starts_with("mod ") || trimmed.starts_with("pub mod ") {
                let base = depth;
                depth += opens - closes;
                skip_from_depth = Some(base);
                continue;
            }
        }

        depth += opens - closes;
        kept.push((idx + 1, line));
    }

    kept
}

fn scan_file(file_name: &str, source: &str) -> Vec<DiscoveredField> {
    let struct_re = Regex::new(r"^\s*(?:pub\s+)?struct\s+(\w+)").expect("valid regex");
    let field_re = Regex::new(r"^\s*pub\s+(?:r#)?(\w+)\s*:\s*(.+?),\s*$").expect("valid regex");
    let enum_re = Regex::new(r"^\s*(?:pub\s+)?enum\s+(\w+)").expect("valid regex");
    // A struct-like enum variant opener, e.g. `Event {` — no `pub`, since
    // Rust does not allow (and does not need) visibility modifiers on enum
    // variant fields: they inherit the enum's own visibility, and serde
    // serializes them unconditionally.
    let variant_open_re = Regex::new(r"^\s*(\w+)\s*\{\s*$").expect("valid regex");
    let variant_field_re = Regex::new(r"^\s*(?:r#)?(\w+)\s*:\s*(.+?),\s*$").expect("valid regex");

    let mut current_struct: Option<String> = None;
    let mut current_enum: Option<String> = None;
    let mut current_variant: Option<String> = None;
    let mut fields = Vec::new();

    for (line_no, line) in strip_test_modules(source) {
        if let Some(caps) = struct_re.captures(line) {
            current_struct = Some(caps[1].to_string());
            current_enum = None;
            current_variant = None;
            continue;
        }
        if let Some(caps) = enum_re.captures(line) {
            current_enum = Some(caps[1].to_string());
            current_struct = None;
            current_variant = None;
            continue;
        }
        if line == "}" {
            current_struct = None;
            current_enum = None;
            current_variant = None;
            continue;
        }

        // Inside an enum body: either walking a struct-like variant's own
        // field list, entering one, or skipping a tuple/unit variant line.
        if let Some(enum_name) = &current_enum {
            if let Some(variant_name) = &current_variant {
                if line.trim() == "}," {
                    current_variant = None;
                    continue;
                }
                if let Some(caps) = variant_field_re.captures(line) {
                    fields.push(DiscoveredField {
                        file: file_name.to_string(),
                        struct_name: format!("{enum_name}::{variant_name}"),
                        field_name: caps[1].to_string(),
                        type_text: caps[2].trim().to_string(),
                        line: line_no,
                    });
                }
                continue;
            }
            if let Some(caps) = variant_open_re.captures(line) {
                current_variant = Some(caps[1].to_string());
            }
            continue;
        }

        if let Some(struct_name) = &current_struct
            && let Some(caps) = field_re.captures(line)
        {
            fields.push(DiscoveredField {
                file: file_name.to_string(),
                struct_name: struct_name.clone(),
                field_name: caps[1].to_string(),
                type_text: caps[2].trim().to_string(),
                line: line_no,
            });
        }
    }

    fields
}

/// Walks every `.rs` file directly under `dtos/` and scans it. Not
/// recursive: `dtos/` has no subdirectories today, and a subdirectory
/// silently escaping this scan would be a real gap, so this asserts there
/// is none rather than quietly ignoring the possibility.
fn discover_all_fields() -> Vec<DiscoveredField> {
    let dir = dtos_dir();
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("must be able to read {}: {e}", dir.display()));

    let mut all_fields = Vec::new();
    let mut file_count = 0;

    for entry in entries {
        let entry = entry.expect("readable dir entry");
        let path = entry.path();

        assert!(
            !path.is_dir(),
            "dtos/ gained a subdirectory ({}) that this scan does not walk into — \
             extend discover_all_fields to recurse before trusting this test again",
            path.display()
        );

        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }

        file_count += 1;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf-8 file name")
            .to_string();
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("must be able to read {}: {e}", path.display()));
        all_fields.extend(scan_file(&file_name, &source));
    }

    assert!(
        file_count >= 16,
        "expected at least 16 .rs files under dtos/ (exploration counted 16 as of S3's \
         grounding); found {file_count} — did the directory move or get renamed?"
    );

    all_fields
}

fn is_exempt(
    field: &DiscoveredField,
) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    EXEMPTIONS.iter().find(|(file, s, name, _)| {
        *file == field.file && *s == field.struct_name && *name == field.field_name
    })
}

/// T2.2/T2.3: every date-named field is `chrono::DateTime<chrono::Utc>`
/// (bare or `Option<..>`), unless explicitly and narrowly exempted above.
#[test]
fn every_date_named_field_is_chrono_datetime_utc() {
    let fields = discover_all_fields();

    let date_named: Vec<&DiscoveredField> = fields.iter().filter(|f| f.is_date_named()).collect();

    assert!(
        date_named.len() >= 80,
        "expected at least 80 date-named fields across dtos/ (exploration counted 87 as of \
         S3's grounding); found {} — the scan may have regressed",
        date_named.len()
    );

    let mut offenders = Vec::new();

    for field in &date_named {
        if field.is_conformant_type() {
            continue;
        }
        if is_exempt(field).is_some() {
            continue;
        }
        offenders.push(format!(
            "{}:{} {}::{} is `{}`, expected `{CONFORMANT_TYPE}` or `{CONFORMANT_TYPE_OPTION}`",
            field.file, field.line, field.struct_name, field.field_name, field.type_text
        ));
    }

    assert!(
        offenders.is_empty(),
        "date-shaped field(s) not typed as chrono::DateTime<chrono::Utc>:\n{}",
        offenders.join("\n")
    );
}

/// T2.1: the exemption list is checked both ways — every entry must still
/// correspond to a real, still-non-conformant field with the recorded
/// shape, or it is stale and must be removed/updated.
#[test]
fn exemptions_are_still_accurate() {
    let fields = discover_all_fields();
    let by_key: BTreeSet<(String, String, String)> = fields
        .iter()
        .map(|f| (f.file.clone(), f.struct_name.clone(), f.field_name.clone()))
        .collect();

    for (file, struct_name, field_name, _reason) in EXEMPTIONS {
        assert!(
            by_key.contains(&(
                file.to_string(),
                struct_name.to_string(),
                field_name.to_string()
            )),
            "exemption ({file}, {struct_name}, {field_name}) no longer matches any scanned \
             field — remove the stale exemption entry"
        );

        let field = fields
            .iter()
            .find(|f| {
                f.file == *file && f.struct_name == *struct_name && f.field_name == *field_name
            })
            .expect("just asserted this exists");

        assert!(
            !field.is_conformant_type(),
            "exemption ({file}, {struct_name}, {field_name}) is now typed \
             `{}`, which IS conformant — remove the now-unnecessary exemption",
            field.type_text
        );
    }
}

/// T2.4's RED proof lives here as a permanent regression test: this asserts
/// the detection logic itself, not the production DTOs. It proves the
/// classifier used by the test above actually rejects a `String`-typed
/// date field rather than passing vacuously — the same "prove the fixture
/// has teeth" discipline PR1's golden-body test used.
#[test]
fn detector_rejects_a_string_typed_date_field() {
    let source = r#"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FakeDto {
    pub created_at: String,
}
"#;
    let fields = scan_file("fake.rs", source);
    let created_at = fields
        .iter()
        .find(|f| f.field_name == "created_at")
        .expect("scan must find the injected field");

    assert!(created_at.is_date_named());
    assert!(
        !created_at.is_conformant_type(),
        "the detector must reject a String-typed date-named field, proving it has teeth; \
         it did not, so it would pass a real regression silently"
    );
}
