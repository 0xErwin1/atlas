#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use clap::Args;

use atlas_api::dtos::DoctorFindingDto;

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output::{self, OutputFormat, TableRow};
use crate::projections::DoctorFindingProjection;

/// Arguments for `atlas doctor`. Flat, no subcommands (E11-S3b design D6.3).
#[derive(Args)]
pub(crate) struct DoctorArgs;

/// Rank used to order findings `Critical, Warning, Info` in the human
/// table. Unknown severities sort last rather than panicking, since a
/// forward-compatible server could in principle add one.
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "warning" => 1,
        "info" => 2,
        _ => 3,
    }
}

/// A stable copy of `findings`, ordered `Critical, Warning, Info` for the
/// human table. `--json` renders the server's own order, unmodified.
fn sorted_by_severity(findings: &[DoctorFindingDto]) -> Vec<DoctorFindingDto> {
    let mut sorted = findings.to_vec();
    sorted.sort_by_key(|finding| severity_rank(&finding.severity));
    sorted
}

/// The CLI's own scriptability contract (design D6.3): exit 1 when any
/// finding is `Critical`, 0 otherwise. The HTTP status is always 200
/// (SHELL-OPS-4) — this exit code carries no server-side meaning.
fn exit_code_for(findings: &[DoctorFindingDto]) -> u8 {
    if findings
        .iter()
        .any(|finding| finding.severity == "critical")
    {
        1
    } else {
        0
    }
}

pub(crate) async fn run(ctx: &Ctx, _args: DoctorArgs) -> Result<(), CliError> {
    let report = ctx.client.platform().doctor().await?;

    match ctx.output {
        OutputFormat::Json => output::print_json(&report)?,
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = sorted_by_severity(&report.findings)
                .into_iter()
                .map(|finding| DoctorFindingProjection::from(finding).row())
                .collect();
            output::print_table(DoctorFindingProjection::headers(), rows)?;
        }
    }

    let exit_code = exit_code_for(&report.findings);
    if exit_code != 0 {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::process::exit(exit_code as i32);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Commands;
    use clap::Parser as ClapParser;

    #[derive(ClapParser)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[test]
    fn doctor_parses_with_no_subcommand() {
        let cli = Cli::try_parse_from(["atlas", "doctor"]).unwrap();
        assert!(matches!(cli.command, Commands::Doctor(_)));
    }

    #[test]
    fn doctor_help_documents_the_exit_code_contract() {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let doctor_cmd = cmd
            .find_subcommand_mut("doctor")
            .expect("doctor subcommand must exist");
        let help = doctor_cmd.render_long_help().to_string();
        assert!(
            help.contains("Critical"),
            "doctor --help must document the Critical-finding exit-code contract: {help}"
        );
    }

    fn finding(component: &str, severity: &str) -> DoctorFindingDto {
        DoctorFindingDto {
            component: component.to_string(),
            severity: severity.to_string(),
            finding: format!("{component} finding"),
            action: "investigate".to_string(),
        }
    }

    #[test]
    fn exit_code_is_zero_with_no_findings() {
        assert_eq!(exit_code_for(&[]), 0);
    }

    #[test]
    fn exit_code_is_zero_with_only_warning_and_info() {
        let findings = vec![finding("acta", "warning"), finding("custos", "info")];
        assert_eq!(exit_code_for(&findings), 0);
    }

    #[test]
    fn exit_code_is_one_with_any_critical_finding() {
        let findings = vec![finding("acta", "warning"), finding("custos", "critical")];
        assert_eq!(exit_code_for(&findings), 1);
    }

    #[test]
    fn sorted_by_severity_orders_critical_then_warning_then_info() {
        let findings = vec![
            finding("a", "info"),
            finding("b", "critical"),
            finding("c", "warning"),
        ];

        let sorted = sorted_by_severity(&findings);

        assert_eq!(
            sorted
                .iter()
                .map(|f| f.severity.as_str())
                .collect::<Vec<_>>(),
            vec!["critical", "warning", "info"]
        );
    }

    #[test]
    fn sorted_by_severity_is_stable_within_the_same_severity() {
        let findings = vec![finding("a", "warning"), finding("b", "warning")];

        let sorted = sorted_by_severity(&findings);

        assert_eq!(sorted[0].component, "a");
        assert_eq!(sorted[1].component, "b");
    }
}
