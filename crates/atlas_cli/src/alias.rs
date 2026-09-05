//! Component-prefixed CLI aliases as an anchored argv rewrite (design D4).
//!
//! `atlas acta docs list` is rewritten to `atlas docs list` *before*
//! `Cli::try_parse_from` ever sees it. There is no second clap tree: the
//! alias is equivalent to the short form by construction, because the
//! rewritten argv and the short-form argv are the same bytes reaching the
//! same parser (design D4's central claim).
//!
//! The rewrite is anchored (D4.2), not a plain `contains`: the first
//! non-flag token after argv[0] is a prefix candidate only when
//!
//! (a) it equals a component name (`acta` / `custos` / `platform`),
//! (b) the next token is a command declared to that component in
//!     [`crate::component::COMMAND_COMPONENTS`], and
//! (c) the token preceding it is not a value-taking global flag.
//!
//! The set in (c) is read from `Cli::command()`'s own parsed global
//! arguments (`Arg::get_action().takes_values()`), never hardcoded, so
//! `atlas --base-url acta docs list` — a base URL that happens to be the
//! string `acta` — is left untouched: `acta` is `--base-url`'s value, not a
//! component prefix.
//!
//! A wrong prefix (`atlas custos docs list`, `docs` is declared `acta`) is
//! an error naming the declared component (design D4.3) rather than a
//! silent no-op — silently accepting it would make the prefix decorative.

use std::ffi::OsString;

use clap::CommandFactory;

use crate::cli::Cli;
use crate::component::{COMMAND_COMPONENTS, Component};

/// The three recognized alias-prefix tokens, paired with the `Component`
/// each denotes. These are argv tokens, not clap subcommand names — no
/// [`COMMAND_COMPONENTS`] row exists for them, and none ever will (T3.7).
const COMPONENT_PREFIXES: [(&str, Component); 3] = [
    ("acta", Component::Acta),
    ("custos", Component::Custos),
    ("platform", Component::Platform),
];

/// The result of scanning argv for a component prefix.
#[derive(Debug)]
pub(crate) enum AliasOutcome {
    /// argv to hand to `Cli::try_parse_from` — unchanged, or with exactly
    /// one leading component token removed.
    Argv(Vec<OsString>),
    /// No positional token follows the component prefix once global flags
    /// (and the value of each value-taking one) are skipped, e.g.
    /// `atlas acta` or `atlas acta --help`: the caller should print that
    /// component's section of the grouped help instead of parsing anything.
    /// A flag before the command token (`atlas acta --json docs list`) does
    /// not make it a help request.
    ComponentHelp(Component),
}

/// A component prefix naming a command declared to a different component
/// (design D4.3). Usage-error shaped: the caller maps this to the CLI's
/// usage exit code (2), matching a clap parse error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WrongComponentPrefix {
    command: String,
    declared: Component,
}

impl std::fmt::Display for WrongComponentPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, article) = component_name_and_article(self.declared);
        write!(
            f,
            "`{command}` is {article} {name} command; use `atlas {name} {command}` or `atlas {command}`",
            command = self.command,
        )
    }
}

fn component_name_and_article(component: Component) -> (&'static str, &'static str) {
    match component {
        Component::Acta => ("acta", "an"),
        Component::Custos => ("custos", "a"),
        Component::Platform => ("platform", "a"),
    }
}

/// Scans `argv` for an anchored component prefix and rewrites it, per
/// design D4.2's three conditions. Returns `argv` unchanged when no
/// condition-satisfying prefix is found.
pub(crate) fn strip_component_prefix(
    argv: Vec<OsString>,
) -> Result<AliasOutcome, WrongComponentPrefix> {
    let Some(anchor) = find_anchor_index(&argv) else {
        return Ok(AliasOutcome::Argv(argv));
    };

    let Some(prefix_component) = argv
        .get(anchor)
        .and_then(|token| token.to_str())
        .and_then(component_named)
    else {
        return Ok(AliasOutcome::Argv(argv));
    };

    let command_token = next_positional_index(&argv, anchor + 1)
        .and_then(|index| argv.get(index))
        .and_then(|token| token.to_str());

    match command_token {
        None => Ok(AliasOutcome::ComponentHelp(prefix_component)),
        Some(next) => match declared_component(next) {
            None => Ok(AliasOutcome::Argv(argv)),
            Some(declared) if declared == prefix_component => {
                let mut rewritten = argv;
                rewritten.remove(anchor);
                Ok(AliasOutcome::Argv(rewritten))
            }
            Some(declared) => Err(WrongComponentPrefix {
                command: next.to_string(),
                declared,
            }),
        },
    }
}

fn component_named(name: &str) -> Option<Component> {
    COMPONENT_PREFIXES
        .iter()
        .find(|(prefix, _)| *prefix == name)
        .map(|(_, component)| *component)
}

fn declared_component(command_name: &str) -> Option<Component> {
    COMMAND_COMPONENTS
        .iter()
        .find(|(name, _)| *name == command_name)
        .map(|(_, component)| *component)
}

/// The index of the first non-flag token at or after `argv[1]`. `argv[0]`
/// is the program name and is never itself a candidate.
fn find_anchor_index(argv: &[OsString]) -> Option<usize> {
    next_positional_index(argv, 1)
}

/// The index of the first non-flag token at or after `start`, skipping over
/// the separate value of any value-taking global flag along the way (design
/// D4.2 condition (c)). `--` ends the scan. This is the single flag-skipping
/// rule shared by the anchor scan and the post-prefix command scan.
fn next_positional_index(argv: &[OsString], start: usize) -> Option<usize> {
    let value_taking_flags = global_value_taking_flag_names();

    let mut index = start;
    while index < argv.len() {
        let token = argv.get(index).and_then(|t| t.to_str()).unwrap_or_default();

        if token == "--" {
            return None;
        }

        if let Some(flag_name) = token.strip_prefix("--") {
            let takes_separate_value =
                !flag_name.contains('=') && value_taking_flags.iter().any(|f| f == flag_name);
            index += if takes_separate_value { 2 } else { 1 };
            continue;
        }

        if token.starts_with('-') && token.len() > 1 {
            index += 1;
            continue;
        }

        return Some(index);
    }

    None
}

/// The long names of every global flag whose `ArgAction` takes a value,
/// read from `Cli::command()`'s own parsed argument definitions — never
/// hardcoded (design D4.2).
fn global_value_taking_flag_names() -> Vec<String> {
    Cli::command()
        .get_arguments()
        .filter(|arg| arg.is_global_set() && arg.get_action().takes_values())
        .filter_map(|arg| arg.get_long().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<OsString> {
        tokens.iter().map(OsString::from).collect()
    }

    /// T3.1 — for every one of the 28 declared commands, the prefixed
    /// argv rewrites to argv byte-identical to the short form, and both
    /// parse to the same `Commands` variant.
    #[test]
    fn alias_rewrite_is_byte_and_parse_equivalent_for_every_command() {
        for (command, component) in COMMAND_COMPONENTS {
            let (prefix, _) = COMPONENT_PREFIXES
                .iter()
                .find(|(_, c)| c == component)
                .unwrap_or_else(|| panic!("{command} has no component prefix"));

            let prefixed = argv(&["atlas", prefix, command]);
            let short = argv(&["atlas", command]);

            let rewritten = match strip_component_prefix(prefixed) {
                Ok(AliasOutcome::Argv(argv)) => argv,
                Ok(AliasOutcome::ComponentHelp(_)) => {
                    panic!("{command}: expected a rewritten argv, got a component-help outcome")
                }
                Err(e) => panic!("{command}: unexpected wrong-prefix error: {e}"),
            };

            assert_eq!(
                rewritten, short,
                "{command}: prefixed argv must rewrite byte-identical to the short form"
            );

            // Neither argv necessarily carries a leaf command's own
            // required args, so parsing may succeed or fail — what must
            // be identical is the outcome itself, proving the rewritten
            // and short argvs parse to the same `Commands` value (or the
            // same parse error) by construction.
            let parse = |argv: Vec<OsString>| {
                Cli::command()
                    .try_get_matches_from(argv)
                    .map_err(|e| e.to_string())
            };

            assert_eq!(
                parse(rewritten),
                parse(short),
                "{command}: rewritten argv must parse to the same value as the short form"
            );
        }
    }

    /// T3.2 — the anchoring probe (D4.2): `acta` immediately after
    /// `--base-url` is that flag's value, not a component prefix, because
    /// `--base-url` is a value-taking global flag.
    #[test]
    fn a_component_name_following_a_value_taking_global_flag_is_left_untouched() {
        let input = argv(&["atlas", "--base-url", "acta", "docs", "list"]);
        let outcome = strip_component_prefix(input.clone())
            .unwrap_or_else(|e| panic!("unexpected wrong-prefix error: {e}"));

        match outcome {
            AliasOutcome::Argv(argv) => {
                assert_eq!(argv, input, "argv must be returned untouched");
            }
            AliasOutcome::ComponentHelp(_) => {
                panic!("expected argv to be returned untouched, got a component-help outcome")
            }
        }
    }

    /// T3.3 — the wrong-prefix probe (D4.3): `docs` is declared `acta`,
    /// not `custos`.
    #[test]
    fn a_command_prefixed_with_the_wrong_component_errors_naming_the_declared_one() {
        let input = argv(&["atlas", "custos", "docs", "list"]);
        let err = strip_component_prefix(input).expect_err("expected a wrong-prefix error");

        assert_eq!(err.declared, Component::Acta);
        let message = err.to_string();
        assert!(
            message.contains("acta"),
            "error must name the declared component: {message:?}"
        );
        assert!(
            message.contains("atlas acta docs") || message.contains("atlas docs"),
            "error must suggest a working invocation: {message:?}"
        );
    }

    /// T3.6 — `atlas <component>` alone (no command) is a component-help
    /// request, not a rewrite.
    #[test]
    fn a_component_name_alone_is_a_component_help_outcome() {
        for (prefix, component) in COMPONENT_PREFIXES {
            let input = argv(&["atlas", prefix]);
            match strip_component_prefix(input).unwrap_or_else(|e| panic!("{e}")) {
                AliasOutcome::ComponentHelp(actual) => assert_eq!(actual, component),
                AliasOutcome::Argv(_) => panic!("{prefix} alone must be a component-help outcome"),
            }
        }
    }

    /// A component name given with only trailing flags (no command token)
    /// is likewise a component-help request.
    #[test]
    fn a_component_name_followed_only_by_flags_is_a_component_help_outcome() {
        for input in [
            argv(&["atlas", "acta", "--help"]),
            argv(&["atlas", "acta", "-h"]),
        ] {
            match strip_component_prefix(input).unwrap_or_else(|e| panic!("{e}")) {
                AliasOutcome::ComponentHelp(component) => assert_eq!(component, Component::Acta),
                AliasOutcome::Argv(_) => panic!("expected a component-help outcome"),
            }
        }
    }

    /// Global flags between the prefix and the command are skipped exactly
    /// as the anchor scan skips them: the prefix is removed and every flag
    /// stays in place and in order.
    #[test]
    fn global_flags_between_the_prefix_and_the_command_are_kept_in_place() {
        let cases = [
            (
                &["atlas", "acta", "--base-url", "http://x", "docs", "list"][..],
                &["atlas", "--base-url", "http://x", "docs", "list"][..],
            ),
            (
                &["atlas", "acta", "--json", "docs", "list"][..],
                &["atlas", "--json", "docs", "list"][..],
            ),
        ];

        for (input, expected) in cases {
            match strip_component_prefix(argv(input)).unwrap_or_else(|e| panic!("{e}")) {
                AliasOutcome::Argv(rewritten) => assert_eq!(rewritten, argv(expected)),
                AliasOutcome::ComponentHelp(_) => {
                    panic!("{input:?}: expected a rewritten argv, got a component-help outcome")
                }
            }
        }
    }

    /// The wrong-prefix check applies to the command found after global
    /// flags too.
    #[test]
    fn a_wrong_prefix_before_global_flags_still_errors_naming_the_declared_one() {
        let input = argv(&["atlas", "custos", "--base-url", "http://x", "docs", "list"]);
        let err = strip_component_prefix(input).expect_err("expected a wrong-prefix error");

        assert_eq!(err.declared, Component::Acta);
        assert!(err.to_string().contains("acta"));
    }

    /// A component name followed by a token that names no real command is
    /// left untouched: clap reports the real parse error on it.
    #[test]
    fn a_component_name_followed_by_an_unknown_token_is_left_untouched() {
        let input = argv(&["atlas", "acta", "not-a-command"]);
        match strip_component_prefix(input.clone()).unwrap_or_else(|e| panic!("{e}")) {
            AliasOutcome::Argv(argv) => assert_eq!(argv, input),
            AliasOutcome::ComponentHelp(_) => panic!("expected argv to be returned untouched"),
        }
    }

    /// argv with no component prefix at all is returned untouched.
    #[test]
    fn argv_without_a_component_prefix_is_untouched() {
        let input = argv(&["atlas", "docs", "list"]);
        match strip_component_prefix(input.clone()).unwrap_or_else(|e| panic!("{e}")) {
            AliasOutcome::Argv(argv) => assert_eq!(argv, input),
            AliasOutcome::ComponentHelp(_) => panic!("expected argv to be returned untouched"),
        }
    }

    /// T3.7 — future-collision guard: no current `COMMAND_COMPONENTS` name
    /// equals a component prefix. If one ever did, the anchoring rule
    /// above would be ambiguous for it.
    #[test]
    fn no_declared_command_name_collides_with_a_component_prefix() {
        for (command, _) in COMMAND_COMPONENTS {
            assert!(
                component_named(command).is_none(),
                "{command} collides with a component prefix name"
            );
        }
    }

    #[test]
    fn a_double_dash_ends_the_scan_and_never_yields_an_anchor() {
        let argv: Vec<OsString> = ["atlas", "--", "acta", "docs", "list"]
            .into_iter()
            .map(OsString::from)
            .collect();

        assert_eq!(find_anchor_index(&argv), None);
        match strip_component_prefix(argv.clone()).unwrap() {
            AliasOutcome::Argv(rewritten) => assert_eq!(rewritten, argv),
            AliasOutcome::ComponentHelp(_) => {
                unreachable!("a double dash never selects a component")
            }
        }
    }
}
