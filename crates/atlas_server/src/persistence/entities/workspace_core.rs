use atlas_domain::entities::workspace_core::{
    AppliesTo, Folder, Project, PropertyDefinition, PropertyKind,
};
use atlas_domain::ids::{FolderId, ProjectId, PropertyDefinitionId, UserId, WorkspaceId};
use atlas_domain::permissions::{Visibility, VisibilityRole};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod property_definition {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "property_definitions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub key: String,
        pub name: String,
        pub kind: String,
        pub options: Option<Json>,
        pub applies_to: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod project {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "projects")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub name: String,
        pub slug: String,
        pub task_prefix: String,
        pub next_task_number: i32,
        pub visibility: String,
        pub visibility_role: Option<String>,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod folder {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "folders")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub project_id: Option<Uuid>,
        pub parent_folder_id: Option<Uuid>,
        pub name: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn property_definition_from(
    m: property_definition::Model,
) -> Result<PropertyDefinition, String> {
    let kind = match m.kind.as_str() {
        "text" => PropertyKind::Text,
        "number" => PropertyKind::Number,
        "boolean" => PropertyKind::Boolean,
        "date" => PropertyKind::Date,
        "select" => PropertyKind::Select,
        "multi_select" => PropertyKind::MultiSelect,
        other => return Err(format!("unknown property kind: {other}")),
    };

    let applies_to = match m.applies_to.as_str() {
        "document" => AppliesTo::Document,
        "task" => AppliesTo::Task,
        "both" => AppliesTo::Both,
        other => return Err(format!("unknown applies_to: {other}")),
    };

    Ok(PropertyDefinition {
        id: PropertyDefinitionId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        key: m.key,
        name: m.name,
        kind,
        options: m.options,
        applies_to,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    })
}

pub fn project_from(m: project::Model) -> Project {
    let visibility = visibility_from_cols(&m.visibility, m.visibility_role.as_deref());
    Project {
        id: ProjectId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        name: m.name,
        slug: m.slug,
        task_prefix: m.task_prefix,
        next_task_number: m.next_task_number,
        visibility,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn visibility_from_cols(visibility: &str, visibility_role: Option<&str>) -> Visibility {
    let role = match visibility_role {
        Some("viewer") => VisibilityRole::Viewer,
        _ => VisibilityRole::Editor,
    };
    match visibility {
        "private" => Visibility::Private,
        "public" => Visibility::Public(role),
        _ => Visibility::Workspace(role),
    }
}

pub fn folder_from(m: folder::Model) -> Folder {
    Folder {
        id: FolderId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: m.project_id.map(ProjectId),
        parent_folder_id: m.parent_folder_id.map(FolderId),
        name: m.name,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}
