use crate::ids::{FolderId, ProjectId, PropertyDefinitionId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyKind {
    Text,
    Number,
    Boolean,
    Date,
    Select,
    MultiSelect,
}

impl PropertyKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PropertyKind::Text => "text",
            PropertyKind::Number => "number",
            PropertyKind::Boolean => "boolean",
            PropertyKind::Date => "date",
            PropertyKind::Select => "select",
            PropertyKind::MultiSelect => "multi_select",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppliesTo {
    Document,
    Task,
    Both,
}

impl AppliesTo {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppliesTo::Document => "document",
            AppliesTo::Task => "task",
            AppliesTo::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyDefinition {
    pub id: PropertyDefinitionId,
    pub workspace_id: WorkspaceId,
    pub key: String,
    pub name: String,
    pub kind: PropertyKind,
    pub options: Option<serde_json::Value>,
    pub applies_to: AppliesTo,
    pub created_by_user_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewPropertyDefinition {
    pub key: String,
    pub name: String,
    pub kind: PropertyKind,
    pub options: Option<serde_json::Value>,
    pub applies_to: AppliesTo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub slug: String,
    pub task_prefix: String,
    pub next_task_number: i32,
    pub created_by_user_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewProject {
    pub name: String,
    pub slug: String,
    pub task_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub parent_folder_id: Option<FolderId>,
    pub name: String,
    pub created_by_user_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewFolder {
    pub project_id: Option<ProjectId>,
    pub parent_folder_id: Option<FolderId>,
    pub name: String,
}
