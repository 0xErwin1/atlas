//! Derived, rendered `--help` grouping (design D1).
//!
//! `atlas --help`'s flat subcommand list is replaced by three headed
//! sections — Acta, Custos, Platform — built by iterating the parsed
//! `clap::Command`'s own subcommands and looking each one up in
//! `component::COMMAND_COMPONENTS` (PR1). No `hide` attribute is used
//! anywhere and the clap subcommand tree itself is untouched: only the
//! rendered help text changes, which keeps `clap_complete`'s generated
//! completions byte-identical (`main.rs`'s `bash_and_zsh_completions_are_
//! byte_identical_to_the_pre_grouping_snapshot`).
//!
//! Spike finding (T2.1): clap 4.6's `write_subcommands`
//! (`clap_builder::output::help_template`) renders every subcommand under a
//! single heading — `subcommand_help_heading` sets one heading for the
//! *whole* subcommand list, and per-`Arg` `help_heading` groups arguments,
//! not subcommands. Neither primitive groups subcommands per-command, so
//! this module renders the three sections itself into a custom
//! `help_template`, which works regardless of that clap limitation
//! (design R1).
//!
//! A derive-time `#[command(help_template = ...)]` cannot compute this
//! template: the expression given to that attribute has no access to the
//! command's own fully-built subcommand list (it runs as one step of
//! building that same list), so the grouped template is instead applied
//! once, at runtime, after `Cli::command()` returns — [`build_command`] is
//! the single entry point `main.rs` and every test in this module use.
//!
//! The sections are rendered from a *built* probe command, not from the
//! raw `Cli::command()` tree: clap inserts its auto-generated `help`
//! pseudo-subcommand only in `Command::build`, so reading the unbuilt tree
//! drops `help` from the listing while `atlas help` keeps working. That
//! pseudo-subcommand is clap's, not one of ours, so it has no
//! `COMMAND_COMPONENTS` row (the table and PR1's derivation audit stay at
//! 28 rows); [`component_of`] places it under Platform by rule and the
//! listing reuses clap's own `about` text for it.

use clap::{Command, CommandFactory};

use crate::cli::Cli;
use crate::component::{COMMAND_COMPONENTS, Component};

/// The three section headings, in the order design D1.3 pins them, each
/// paired with the component whose commands it lists.
pub(crate) const HEADINGS: [(&str, Component); 3] = [
    ("Acta commands", Component::Acta),
    ("Custos commands", Component::Custos),
    ("Platform commands", Component::Platform),
];

/// Builds the `atlas` root command with the grouped `--help` template
/// wired in. This is the command `main.rs` parses argv against; every
/// per-subcommand `--help` (e.g. `atlas docs --help`) is unaffected, since
/// only the root command's template changes.
pub(crate) fn build_command() -> Command {
    let template = render_template(&built_probe());
    Cli::command().help_template(template)
}

/// The name clap gives its auto-generated help pseudo-subcommand.
const CLAP_HELP_SUBCOMMAND: &str = "help";

/// A fully built copy of the root command, used only to read the final
/// subcommand list (with clap's `help` pseudo-subcommand present). The
/// command returned by [`build_command`] is a separate, unbuilt tree, so
/// callers see exactly what `Cli::command()` declares and clap builds it
/// once, at parse time, as it always did.
fn built_probe() -> Command {
    let mut probe = Cli::command();
    probe.build();
    probe
}

fn render_template(cmd: &Command) -> String {
    let sections = render_grouped_sections(cmd);
    format!(
        "{{before-help}}{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{sections}\n\n{{options}}{{after-help}}"
    )
}

/// Maps a rendered subcommand name to its section. Every declared command
/// is looked up in `COMMAND_COMPONENTS`; clap's own `help` pseudo-subcommand
/// is placed under Platform by rule, since it is the CLI itself rather than
/// a component call. Any other unmapped name is a bug: PR1's totality test
/// pins the table against the declared variants, so reaching it means a
/// subcommand exists that neither the table nor this rule knows about.
fn component_of(name: &str) -> Component {
    if name == CLAP_HELP_SUBCOMMAND {
        return Component::Platform;
    }

    COMMAND_COMPONENTS
        .iter()
        .find(|(row_name, _)| *row_name == name)
        .map(|(_, component)| *component)
        .unwrap_or_else(|| {
            unreachable!(
                "{name} has no COMMAND_COMPONENTS row and is not clap's help pseudo-subcommand"
            )
        })
}

fn render_grouped_sections(cmd: &Command) -> String {
    let name_width = cmd
        .get_subcommands()
        .map(|sub| sub.get_name().len())
        .max()
        .unwrap_or(0)
        + 2;

    let sections: Vec<String> = HEADINGS
        .iter()
        .map(|(heading, component)| render_section(cmd, heading, *component, name_width))
        .collect();

    sections.join("\n\n")
}

fn render_section(cmd: &Command, heading: &str, component: Component, name_width: usize) -> String {
    let lines: Vec<String> = cmd
        .get_subcommands()
        .filter(|sub| component_of(sub.get_name()) == component)
        .map(|sub| {
            let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
            format!("  {:<name_width$}{about}", sub.get_name())
        })
        .collect();

    format!("{heading}:\n{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::EXIT_CODES_HELP;

    fn rendered_help() -> String {
        build_command().render_long_help().to_string()
    }

    /// The exact literal block this module inserts into the help template
    /// in place of clap's flat "Commands:" list — the same text
    /// [`render_template`] substitutes into `{sections}`. Reading it
    /// directly (rather than re-parsing the final rendered help) avoids
    /// having to separate it from `{options}`'s own line-wrapped output,
    /// which clap lays out dynamically and which this module never touches.
    fn grouped_sections_text() -> String {
        render_grouped_sections(&built_probe())
    }

    /// Every line in the grouped-sections block starts with two spaces
    /// followed by a command name token — there is no other kind of line
    /// in this block (headings end with `:` on their own line and are
    /// filtered out by the two-space prefix check).
    fn command_names_in_rendered_order(sections: &str) -> Vec<String> {
        sections
            .lines()
            .filter(|line| line.starts_with("  "))
            .filter_map(|line| line.split_whitespace().next())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_three_headings_appear_exactly_once_each_and_in_order() {
        let help = rendered_help();

        let positions: Vec<usize> = HEADINGS
            .iter()
            .map(|(heading, _)| {
                let marker = format!("{heading}:");
                help.find(&marker)
                    .unwrap_or_else(|| panic!("heading {heading:?} missing from rendered help"))
            })
            .collect();

        assert!(
            positions.windows(2).all(|pair| pair.first() < pair.get(1)),
            "headings out of D1.3 order: {positions:?}"
        );

        for (heading, _) in HEADINGS {
            let marker = format!("{heading}:");
            assert_eq!(
                help.matches(&marker).count(),
                1,
                "{heading} must appear exactly once"
            );
        }
    }

    #[test]
    fn every_declared_command_plus_clap_help_appears_exactly_once_across_the_sections() {
        let sections = grouped_sections_text();

        let mut rendered = command_names_in_rendered_order(&sections);
        rendered.sort_unstable();

        let mut expected: Vec<String> = COMMAND_COMPONENTS
            .iter()
            .map(|(name, _)| name.to_string())
            .chain(std::iter::once(CLAP_HELP_SUBCOMMAND.to_string()))
            .collect();
        expected.sort_unstable();

        assert_eq!(
            rendered.len(),
            29,
            "rendered help must list the 28 declared commands plus clap's help pseudo-subcommand"
        );
        assert_eq!(
            rendered, expected,
            "rendered help must list exactly the 28 declared commands plus help, each exactly once"
        );
    }

    #[test]
    fn clap_help_pseudo_subcommand_is_listed_under_platform_with_clap_about_text() {
        let sections = grouped_sections_text();
        let platform_start = sections
            .find("Platform commands:")
            .expect("Platform heading missing");
        let platform_section = &sections[platform_start..];

        let clap_about = built_probe()
            .find_subcommand(CLAP_HELP_SUBCOMMAND)
            .and_then(|sub| sub.get_about().map(ToString::to_string))
            .expect("clap must expose its help pseudo-subcommand with an about text");

        let help_line = platform_section
            .lines()
            .find(|line| line.split_whitespace().next() == Some(CLAP_HELP_SUBCOMMAND))
            .expect("help must be listed under the Platform section");

        assert_eq!(
            help_line
                .trim_start()
                .trim_start_matches(CLAP_HELP_SUBCOMMAND)
                .trim_start(),
            clap_about,
            "help must carry clap's own about text"
        );
        assert!(
            rendered_help().contains(help_line),
            "the help line must reach the final rendered --help"
        );
    }

    #[test]
    fn an_unmapped_subcommand_name_is_a_loud_failure() {
        let outcome = std::panic::catch_unwind(|| component_of("not-a-command"));
        assert!(
            outcome.is_err(),
            "unmapped names must panic, not be silently placed"
        );
    }

    #[test]
    fn docs_appears_under_the_acta_section() {
        let help = rendered_help();
        let acta_start = help.find("Acta commands:").expect("Acta heading missing");
        let custos_start = help
            .find("Custos commands:")
            .expect("Custos heading missing");
        let acta_section = &help[acta_start..custos_start];

        assert!(
            acta_section
                .lines()
                .any(|line| line.split_whitespace().next() == Some("docs")),
            "docs must be listed under the Acta section"
        );
    }

    #[test]
    fn version_appears_under_the_platform_section() {
        let help = rendered_help();
        let platform_start = help
            .find("Platform commands:")
            .expect("Platform heading missing");
        let platform_section = &help[platform_start..];

        assert!(
            platform_section
                .lines()
                .any(|line| line.split_whitespace().next() == Some("version")),
            "version must be listed under the Platform section (design D2)"
        );
    }

    #[test]
    fn exit_codes_help_block_survives_grouping() {
        let help = rendered_help();
        for line in EXIT_CODES_HELP.lines() {
            assert!(
                help.contains(line),
                "grouped help must still contain the exit-codes line: {line:?}"
            );
        }
    }

    #[test]
    fn no_hide_attribute_is_needed_to_render_the_grouped_sections() {
        let cmd = build_command();
        let hidden: Vec<&str> = cmd
            .get_subcommands()
            .filter(|sub| sub.is_hide_set())
            .map(|sub| sub.get_name())
            .collect();
        assert!(
            hidden.is_empty(),
            "no subcommand may be hidden by the grouped help: {hidden:?}"
        );
    }
}
