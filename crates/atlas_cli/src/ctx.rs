use atlas_client::AtlasClient;

use crate::error::CliError;
use crate::output::OutputFormat;

pub(crate) struct Ctx {
    pub(crate) client: AtlasClient,
    pub(crate) output: OutputFormat,
    pub(crate) workspace: Option<String>,
}

impl Ctx {
    /// Returns the effective workspace slug for the current command.
    ///
    /// Resolution order: per-command `--workspace` argument → global `ctx.workspace`.
    /// Returns `CliError::Validation` when neither is set.
    pub(crate) fn require_workspace<'a>(
        &'a self,
        per_cmd: Option<&'a str>,
    ) -> Result<&'a str, CliError> {
        per_cmd
            .or(self.workspace.as_deref())
            .ok_or_else(|| CliError::Validation("a workspace is required; pass --workspace".into()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn make_ctx(workspace: Option<&str>) -> Ctx {
        Ctx {
            client: AtlasClient::new("http://localhost:8080"),
            output: OutputFormat::Human,
            workspace: workspace.map(str::to_owned),
        }
    }

    #[test]
    fn per_cmd_workspace_takes_priority() {
        let ctx = make_ctx(Some("ctx-ws"));
        let ws = ctx.require_workspace(Some("cmd-ws")).unwrap();
        assert_eq!(ws, "cmd-ws");
    }

    #[test]
    fn ctx_workspace_used_when_no_per_cmd() {
        let ctx = make_ctx(Some("ctx-ws"));
        let ws = ctx.require_workspace(None).unwrap();
        assert_eq!(ws, "ctx-ws");
    }

    #[test]
    fn error_when_neither_provided() {
        let ctx = make_ctx(None);
        let result = ctx.require_workspace(None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CliError::Validation(_)));
    }
}
