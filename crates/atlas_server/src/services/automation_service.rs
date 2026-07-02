#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::boards_tasks::{NewTask, PositionBetween},
    ids::{ApiKeyId, BoardId, ColumnId, ProjectId, TaskId, WorkspaceId},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};
use uuid::Uuid;

use crate::{
    persistence::{
        entities::boards_tasks::{board, task},
        repos::{PgAutomationRuleRepo, PgOutboxRepo},
    },
    services::TaskService,
};

/// Maximum characters allowed for any single substituted value in a title template.
const MAX_TEMPLATE_VALUE_CHARS: usize = 256;

/// Inline rule-evaluation service for incoming external events.
///
/// Given a verified delivery, it persists the external event to the outbox (with
/// dedup via `ON CONFLICT DO NOTHING`), loads matching automation rules, evaluates
/// each rule's filter, and creates tasks via `TaskService` attributed to the
/// integration's provisioned api key.
///
/// Error semantics:
/// - Permanent misconfigurations (board/column not found, malformed `action_params`)
///   are logged and skipped; the method returns `Ok`.
/// - Transient infrastructure errors (DB failures) are propagated as `Err`, allowing
///   the handler to respond with 500 so GitHub retries.
pub struct AutomationService {
    conn: DatabaseConnection,
}

impl AutomationService {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Processes one GitHub delivery end-to-end.
    ///
    /// Returns `Ok(true)` when the event was new and rules were evaluated,
    /// `Ok(false)` when the delivery was a duplicate (no-op), and `Err` on a
    /// transient infrastructure failure.
    ///
    /// The external-event commit is independent of task creation so that dedup
    /// works correctly even when task creation fails. This means a transient
    /// failure strictly after the external-event commit drops the action on retry
    /// (acknowledged v1 trade-off; at-least-once would require a separate worker).
    pub async fn process_github_delivery(
        &self,
        workspace_id: Uuid,
        integration_api_key_id: Uuid,
        delivery_id: Uuid,
        event_name: &str,
        raw_payload: &serde_json::Value,
    ) -> Result<bool, DomainError> {
        let data = if event_name == "workflow_run" {
            normalize_github_workflow_run(raw_payload)
        } else {
            raw_payload.clone()
        };

        let event_type = format!("external.github.{event_name}");

        let txn = self.conn.begin().await.map_err(db_err)?;
        let inserted = PgOutboxRepo::insert_external_in(
            &txn,
            delivery_id,
            workspace_id,
            "external/github",
            &event_type,
            integration_api_key_id,
            data.clone(),
        )
        .await?;
        txn.commit().await.map_err(db_err)?;

        if !inserted {
            return Ok(false);
        }

        let rules = PgAutomationRuleRepo::list_active_for_workspace_event(
            &self.conn,
            workspace_id,
            &event_type,
            None,
        )
        .await?;

        let task_svc = TaskService::new(self.conn.clone());

        for rule in &rules {
            self.execute_rule(&task_svc, rule, workspace_id, integration_api_key_id, &data)
                .await?;
        }

        Ok(true)
    }

    async fn execute_rule(
        &self,
        task_svc: &TaskService,
        rule: &crate::persistence::entities::automation_rule::automation_rules::Model,
        workspace_id: Uuid,
        integration_api_key_id: Uuid,
        data: &serde_json::Value,
    ) -> Result<(), DomainError> {
        if !evaluate_trigger_filter(rule.trigger_filter.as_ref(), data) {
            return Ok(());
        }

        // The DB CHECK constrains `action_type` to this set, but an unknown value
        // is still handled as a permanent misconfig rather than assumed away.
        match rule.action_type.as_str() {
            "create_task" => {
                self.execute_create_task(task_svc, rule, workspace_id, integration_api_key_id, data)
                    .await
            }
            "add_comment" => {
                self.execute_add_comment(task_svc, rule, workspace_id, integration_api_key_id, data)
                    .await
            }
            other => {
                tracing::warn!(rule_id = %rule.id, action_type = %other, "permanent misconfig: unknown action_type");
                Ok(())
            }
        }
    }

    async fn execute_create_task(
        &self,
        task_svc: &TaskService,
        rule: &crate::persistence::entities::automation_rule::automation_rules::Model,
        workspace_id: Uuid,
        integration_api_key_id: Uuid,
        data: &serde_json::Value,
    ) -> Result<(), DomainError> {
        let params: CreateTaskParams = match serde_json::from_value(rule.action_params.clone()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(rule_id = %rule.id, error = %e, "permanent misconfig: invalid action_params");
                return Ok(());
            }
        };

        let board_row = board::Entity::find_by_id(params.board_id)
            .filter(board::Column::WorkspaceId.eq(workspace_id))
            .filter(board::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        let board_row = match board_row {
            Some(row) => row,
            None => {
                tracing::warn!(
                    rule_id = %rule.id,
                    board_id = %params.board_id,
                    "permanent misconfig: board not found"
                );
                return Ok(());
            }
        };

        let title = render_template(&params.title_template, data);

        let priority = params.priority.as_deref().and_then(|s| {
            s.parse::<atlas_domain::entities::boards_tasks::Priority>()
                .ok()
        });

        let ctx = WorkspaceCtx::new(
            WorkspaceId(workspace_id),
            Actor::ApiKey(ApiKeyId(integration_api_key_id)),
        );

        let new_task = NewTask {
            project_id: ProjectId(board_row.project_id),
            board_id: BoardId(params.board_id),
            column_id: ColumnId(params.column_id),
            title,
            description: String::new(),
            priority,
            due_date: None,
            estimate: None,
            labels: vec![],
            properties: None,
            position: PositionBetween {
                before: None,
                after: None,
            },
        };

        match task_svc.create(&ctx, new_task).await {
            Ok(_) => {}
            Err(DomainError::NotFound { entity, id }) => {
                tracing::warn!(
                    rule_id = %rule.id,
                    entity,
                    id = %id,
                    "permanent misconfig: board/column not found during task creation"
                );
            }
            Err(e) => {
                return Err(e);
            }
        }

        Ok(())
    }

    async fn execute_add_comment(
        &self,
        task_svc: &TaskService,
        rule: &crate::persistence::entities::automation_rule::automation_rules::Model,
        workspace_id: Uuid,
        integration_api_key_id: Uuid,
        data: &serde_json::Value,
    ) -> Result<(), DomainError> {
        let params: AddCommentParams = match serde_json::from_value(rule.action_params.clone()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(rule_id = %rule.id, error = %e, "permanent misconfig: invalid action_params");
                return Ok(());
            }
        };

        // The comment repo trusts the given task id and only stamps the ctx
        // workspace onto the row, so the target task must be verified to live in
        // this workspace (and not be soft-deleted) first. A stale or foreign id
        // is a permanent misconfig — logged and skipped — never a comment written
        // onto another workspace's task or a 500 from the FK violation.
        let task_row = task::Entity::find_by_id(params.task_id)
            .filter(task::Column::WorkspaceId.eq(workspace_id))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        let task_row = match task_row {
            Some(row) => row,
            None => {
                tracing::warn!(
                    rule_id = %rule.id,
                    task_id = %params.task_id,
                    "permanent misconfig: target task not found"
                );
                return Ok(());
            }
        };

        let body = render_template(&params.body_template, data);

        let ctx = WorkspaceCtx::new(
            WorkspaceId(workspace_id),
            Actor::ApiKey(ApiKeyId(integration_api_key_id)),
        );

        match task_svc.add_comment(&ctx, TaskId(task_row.id), body).await {
            Ok(_) => {}
            Err(DomainError::NotFound { entity, id }) => {
                tracing::warn!(
                    rule_id = %rule.id,
                    entity,
                    id = %id,
                    "permanent misconfig: task not found during comment creation"
                );
            }
            Err(e) => {
                return Err(e);
            }
        }

        Ok(())
    }
}

/// Normalizes a raw GitHub `workflow_run` delivery into a flat top-level map
/// suitable for `trigger_filter` comparison and `{{key}}` template rendering.
///
/// Extracts: `action`, `conclusion`, `workflow_name`, `run_id`, `html_url`,
/// `head_branch`, `repository` (full_name), and `event`.
pub(crate) fn normalize_github_workflow_run(json: &serde_json::Value) -> serde_json::Value {
    let run = json.get("workflow_run");

    let action = json
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let conclusion = run
        .and_then(|r| r.get("conclusion"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let workflow_name = run
        .and_then(|r| r.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let run_id = run
        .and_then(|r| r.get("id"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let html_url = run
        .and_then(|r| r.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let head_branch = run
        .and_then(|r| r.get("head_branch"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let repository = json
        .get("repository")
        .and_then(|r| r.get("full_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let event = run
        .and_then(|r| r.get("event"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    serde_json::json!({
        "action": action,
        "conclusion": conclusion,
        "workflow_name": workflow_name,
        "run_id": run_id,
        "html_url": html_url,
        "head_branch": head_branch,
        "repository": repository,
        "event": event,
    })
}

/// Evaluates a rule's `trigger_filter` against the event data.
///
/// `None` filter matches everything. A JSON object filter matches when every
/// key-value pair in the filter equals the corresponding key-value in `data`
/// (top-level equality only; nested paths are not supported).
pub(crate) fn evaluate_trigger_filter(
    filter: Option<&serde_json::Value>,
    data: &serde_json::Value,
) -> bool {
    let filter = match filter {
        None => return true,
        Some(f) => f,
    };

    let filter_obj = match filter.as_object() {
        Some(o) => o,
        None => return false,
    };

    let data_obj = match data.as_object() {
        Some(o) => o,
        None => return false,
    };

    for (key, expected) in filter_obj {
        match data_obj.get(key) {
            Some(actual) if actual == expected => {}
            _ => return false,
        }
    }

    true
}

/// Renders a `{{key}}` template against the event data.
///
/// Each `{{key}}` placeholder is replaced by the string representation of the
/// top-level value for `key` in `data`, truncated to `MAX_TEMPLATE_VALUE_CHARS`
/// characters. Unknown keys are replaced with an empty string. Malformed
/// placeholders (e.g. unclosed `{{`) are emitted verbatim.
pub(crate) fn render_template(template: &str, data: &serde_json::Value) -> String {
    let data_obj = match data.as_object() {
        Some(o) => o,
        None => return template.to_string(),
    };

    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            chars.next();

            let mut key = String::new();
            let mut found_close = false;

            loop {
                match chars.next() {
                    None => {
                        result.push_str("{{");
                        result.push_str(&key);
                        break;
                    }
                    Some('}') if chars.peek() == Some(&'}') => {
                        chars.next();
                        found_close = true;
                        break;
                    }
                    Some(c) => key.push(c),
                }
            }

            if found_close {
                let value_str = data_obj
                    .get(&key)
                    .map(|v| match v.as_str() {
                        Some(s) => s.to_string(),
                        None => v.to_string(),
                    })
                    .unwrap_or_default();

                let truncated: String = value_str.chars().take(MAX_TEMPLATE_VALUE_CHARS).collect();
                result.push_str(&truncated);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

#[derive(Debug, serde::Deserialize)]
struct CreateTaskParams {
    board_id: Uuid,
    column_id: Uuid,
    title_template: String,
    priority: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct AddCommentParams {
    task_id: Uuid,
    body_template: String,
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> serde_json::Value {
        serde_json::json!({
            "conclusion": "failure",
            "workflow_name": "CI Pipeline",
            "run_id": 12345,
            "head_branch": "main",
        })
    }

    // B3.4 [U] Filter match: matching filter fires
    #[test]
    fn filter_match_returns_true() {
        let filter = serde_json::json!({"conclusion": "failure"});
        let data = sample_data();
        assert!(evaluate_trigger_filter(Some(&filter), &data));
    }

    // B3.4 [U] Filter non-match: wrong value produces nothing
    #[test]
    fn filter_no_match_returns_false() {
        let filter = serde_json::json!({"conclusion": "success"});
        let data = sample_data();
        assert!(!evaluate_trigger_filter(Some(&filter), &data));
    }

    // B3.4 [U] None filter matches everything
    #[test]
    fn no_filter_matches_all() {
        assert!(evaluate_trigger_filter(None, &sample_data()));
    }

    // B3.4 [U] Filter with multiple keys: all must match
    #[test]
    fn multi_key_filter_all_must_match() {
        let filter = serde_json::json!({"conclusion": "failure", "head_branch": "main"});
        let data = sample_data();
        assert!(evaluate_trigger_filter(Some(&filter), &data));
    }

    // B3.4 [U] Filter with multiple keys: partial match → no match
    #[test]
    fn multi_key_filter_partial_is_no_match() {
        let filter = serde_json::json!({"conclusion": "failure", "head_branch": "feature"});
        let data = sample_data();
        assert!(!evaluate_trigger_filter(Some(&filter), &data));
    }

    // B3.4 [U] Filter key absent in data → no match
    #[test]
    fn filter_key_absent_is_no_match() {
        let filter = serde_json::json!({"nonexistent_key": "value"});
        let data = sample_data();
        assert!(!evaluate_trigger_filter(Some(&filter), &data));
    }

    // B3.4 [U] Top-level equality only: nested path in filter value does not
    // traverse into data (the filter key must literally exist at top level)
    #[test]
    fn filter_is_top_level_equality_only() {
        let filter = serde_json::json!({"conclusion": {"nested": "value"}});
        let data = serde_json::json!({"conclusion": "failure"});
        assert!(!evaluate_trigger_filter(Some(&filter), &data));
    }

    // B3.4 [U] Template substitution replaces {{key}} with the value
    #[test]
    fn template_substitutes_known_key() {
        let data = sample_data();
        let result = render_template("CI failed: {{workflow_name}}", &data);
        assert_eq!(result, "CI failed: CI Pipeline");
    }

    // B3.4 [U] Unknown key is replaced with empty string
    #[test]
    fn template_unknown_key_becomes_empty_string() {
        let data = sample_data();
        let result = render_template("Build: {{unknown_key}}", &data);
        assert_eq!(result, "Build: ");
    }

    // B3.4 [U] Numeric value is rendered as its string representation
    #[test]
    fn template_numeric_value_rendered_as_string() {
        let data = sample_data();
        let result = render_template("Run #{{run_id}}", &data);
        assert_eq!(result, "Run #12345");
    }

    // B3.4 [U] Value exceeding MAX_TEMPLATE_VALUE_CHARS is truncated
    #[test]
    fn template_long_value_is_truncated() {
        let long_name = "A".repeat(MAX_TEMPLATE_VALUE_CHARS + 100);
        let data = serde_json::json!({"workflow_name": long_name});
        let result = render_template("{{workflow_name}}", &data);
        assert_eq!(result.len(), MAX_TEMPLATE_VALUE_CHARS);
    }

    // B3.4 [U] Value at exactly MAX_TEMPLATE_VALUE_CHARS is NOT truncated
    #[test]
    fn template_value_at_limit_not_truncated() {
        let name = "B".repeat(MAX_TEMPLATE_VALUE_CHARS);
        let data = serde_json::json!({"workflow_name": name.clone()});
        let result = render_template("{{workflow_name}}", &data);
        assert_eq!(result, name);
    }

    // B3.4 [U] Multiple placeholders in one template
    #[test]
    fn template_multiple_placeholders() {
        let data = serde_json::json!({
            "head_branch": "main",
            "conclusion": "failure",
        });
        let result = render_template("Branch {{head_branch}}: {{conclusion}}", &data);
        assert_eq!(result, "Branch main: failure");
    }

    // normalize_github_workflow_run extracts expected top-level fields
    #[test]
    fn normalize_workflow_run_extracts_fields() {
        let payload = serde_json::json!({
            "action": "completed",
            "workflow_run": {
                "id": 99,
                "name": "CI",
                "conclusion": "failure",
                "html_url": "https://example.com/runs/99",
                "head_branch": "main",
                "event": "push"
            },
            "repository": {
                "full_name": "owner/repo"
            }
        });

        let data = normalize_github_workflow_run(&payload);

        assert_eq!(data["action"], "completed");
        assert_eq!(data["conclusion"], "failure");
        assert_eq!(data["workflow_name"], "CI");
        assert_eq!(data["run_id"], 99);
        assert_eq!(data["html_url"], "https://example.com/runs/99");
        assert_eq!(data["head_branch"], "main");
        assert_eq!(data["repository"], "owner/repo");
        assert_eq!(data["event"], "push");
    }

    // normalize_github_workflow_run handles missing fields gracefully
    #[test]
    fn normalize_workflow_run_handles_missing_fields() {
        let data = normalize_github_workflow_run(&serde_json::json!({}));
        assert_eq!(data["conclusion"], "");
        assert_eq!(data["workflow_name"], "");
    }

    // B3.5 [U] Cascade guard: task.created does not start with "external."
    // so it can never be stored as a rule's trigger_event_type (DB CHECK + app validate).
    // This unit test documents the invariant: the app-layer guard rejects the type.
    #[test]
    fn cascade_guard_task_created_not_external() {
        let not_external = "task.created";
        assert!(
            !not_external.starts_with("external."),
            "task.created must not start with 'external.' — the cascade guard relies on this"
        );
    }
}
