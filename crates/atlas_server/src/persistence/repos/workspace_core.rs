use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::workspace_core::{
        Folder, NewFolder, NewProject, NewPropertyDefinition, Project, PropertyDefinition,
        UpdateProject,
    },
    ids::{FolderId, ProjectId, PropertyDefinitionId},
    permissions::{Principal, Visibility, VisibilityRole},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter,
};

use crate::persistence::entities::workspace_core::{
    folder, folder_from, project, project_from, property_definition, property_definition_from,
    visibility_from_cols,
};

pub use atlas_domain::ports::workspace_core::{FolderRepo, ProjectRepo, PropertyDefinitionRepo};

fn visibility_to_str(v: &Visibility) -> (&'static str, Option<&'static str>) {
    match v {
        Visibility::Private => ("private", None),
        Visibility::Workspace(r) | Visibility::Public(r) => {
            let vis_str = if matches!(v, Visibility::Workspace(_)) {
                "workspace"
            } else {
                "public"
            };
            let role_str = match r {
                VisibilityRole::Viewer => "viewer",
                VisibilityRole::Editor => "editor",
            };
            (vis_str, Some(role_str))
        }
    }
}

pub struct PgPropertyDefinitionRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl PropertyDefinitionRepo for PgPropertyDefinitionRepo {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewPropertyDefinition,
    ) -> Result<PropertyDefinition, DomainError> {
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let model = property_definition::ActiveModel {
            id: Set(PropertyDefinitionId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            key: Set(new.key),
            name: Set(new.name),
            kind: Set(new.kind.as_str().to_string()),
            options: Set(new.options),
            applies_to: Set(new.applies_to.as_str().to_string()),
            created_by_user_id: Set(created_by_user_id),
            created_by_api_key_id: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map_err(db_err)
            .and_then(|m| property_definition_from(m).map_err(internal_err))
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: PropertyDefinitionId,
    ) -> Result<Option<PropertyDefinition>, DomainError> {
        property_definition::Entity::find_by_id(id.0)
            .filter(property_definition::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(property_definition::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(property_definition_from)
            .transpose()
            .map_err(internal_err)
    }

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<PropertyDefinition>, DomainError> {
        let rows = property_definition::Entity::find()
            .filter(property_definition::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(property_definition::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter()
            .map(|m| property_definition_from(m).map_err(internal_err))
            .collect()
    }

    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        id: PropertyDefinitionId,
    ) -> Result<(), DomainError> {
        let row = property_definition::Entity::find_by_id(id.0)
            .filter(property_definition::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(property_definition::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "property_definition",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

pub struct PgProjectRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl ProjectRepo for PgProjectRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewProject) -> Result<Project, DomainError> {
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let (vis_str, vis_role_str) = visibility_to_str(&new.visibility);
        let model = project::ActiveModel {
            id: Set(ProjectId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            name: Set(new.name),
            slug: Set(new.slug),
            task_prefix: Set(new.task_prefix),
            next_task_number: Set(0),
            visibility: Set(vis_str.to_string()),
            visibility_role: Set(vis_role_str.map(|s| s.to_string())),
            created_by_user_id: Set(created_by_user_id),
            created_by_api_key_id: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(project_from)
            .map_err(db_err)
    }

    async fn list_visible(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        after_id: Option<uuid::Uuid>,
        limit: u64,
    ) -> Result<Vec<Project>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: uuid::Uuid,
            name: String,
            slug: String,
            task_prefix: String,
            next_task_number: i32,
            visibility: String,
            visibility_role: Option<String>,
            created_by_user_id: Option<uuid::Uuid>,
            created_at: chrono::DateTime<chrono::Utc>,
            updated_at: chrono::DateTime<chrono::Utc>,
            deleted_at: Option<chrono::DateTime<chrono::Utc>>,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        // $1 — workspace_id
        values.push(ctx.workspace_id.0.into());

        // $2 — workspace_id repeated for the EXISTS sub-query
        values.push(ctx.workspace_id.0.into());

        // $3 — principal column value (user_id or api_key_id)
        let principal_col;
        match principal {
            Principal::User(uid) => {
                principal_col = "user_id";
                values.push(uid.0.into());
            }
            Principal::ApiKey(kid) => {
                principal_col = "api_key_id";
                values.push(kid.0.into());
            }
        }

        let cursor_cond = if let Some(cursor) = after_id {
            values.push(cursor.into());
            format!("AND p.id > ${}", values.len())
        } else {
            String::new()
        };

        // limit and offset are not user-supplied (they come from pagination config),
        // so it is safe to interpolate them directly.
        let sql = format!(
            r#"
            SELECT p.id, p.workspace_id, p.name, p.slug, p.task_prefix, p.next_task_number,
                   p.visibility, p.visibility_role,
                   p.created_by_user_id, p.created_at, p.updated_at, p.deleted_at
            FROM projects p
            WHERE p.workspace_id = $1
              AND p.deleted_at IS NULL
              AND (
                    p.visibility != 'private'
                    OR EXISTS (
                        SELECT 1 FROM permission_grants
                        WHERE workspace_id = $2
                          AND {principal_col} = $3
                          AND project_id = p.id
                    )
              )
              {cursor_cond}
            ORDER BY p.id
            LIMIT {limit}
            "#,
        );

        let rows = Row::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let visibility = visibility_from_cols(&r.visibility, r.visibility_role.as_deref());
                Project {
                    id: atlas_domain::ids::ProjectId(r.id),
                    workspace_id: atlas_domain::ids::WorkspaceId(r.workspace_id),
                    name: r.name,
                    slug: r.slug,
                    task_prefix: r.task_prefix,
                    next_task_number: r.next_task_number,
                    visibility,
                    created_by_user_id: r.created_by_user_id.map(atlas_domain::ids::UserId),
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                    deleted_at: r.deleted_at,
                }
            })
            .collect())
    }

    async fn update(
        &self,
        ctx: &WorkspaceCtx,
        id: ProjectId,
        update: UpdateProject,
    ) -> Result<Project, DomainError> {
        let row = project::Entity::find_by_id(id.0)
            .filter(project::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(project::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "project",
                id: id.0,
            })?;

        let mut active = row.into_active_model();

        if let Some(name) = update.name {
            active.name = Set(name);
        }

        if let Some(vis) = update.visibility {
            let (vis_str, vis_role_str) = visibility_to_str(&vis);
            active.visibility = Set(vis_str.to_string());
            active.visibility_role = Set(vis_role_str.map(|s| s.to_string()));
        }

        active.updated_at = Set(Utc::now());
        let updated = active.update(&self.conn).await.map_err(db_err)?;
        Ok(project_from(updated))
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: ProjectId,
    ) -> Result<Option<Project>, DomainError> {
        project::Entity::find_by_id(id.0)
            .filter(project::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(project::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(project_from))
            .map_err(db_err)
    }

    async fn find_by_slug(
        &self,
        ctx: &WorkspaceCtx,
        slug: &str,
    ) -> Result<Option<Project>, DomainError> {
        project::Entity::find()
            .filter(project::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(project::Column::Slug.eq(slug))
            .filter(project::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(project_from))
            .map_err(db_err)
    }

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<Project>, DomainError> {
        project::Entity::find()
            .filter(project::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(project::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(project_from).collect())
            .map_err(db_err)
    }

    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: ProjectId,
        name: String,
    ) -> Result<(), DomainError> {
        let row = project::Entity::find_by_id(id.0)
            .filter(project::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(project::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "project",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.name = Set(name);
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: ProjectId) -> Result<(), DomainError> {
        let row = project::Entity::find_by_id(id.0)
            .filter(project::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(project::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "project",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

pub struct PgFolderRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl FolderRepo for PgFolderRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewFolder) -> Result<Folder, DomainError> {
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let model = folder::ActiveModel {
            id: Set(FolderId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            project_id: Set(new.project_id.map(|id| id.0)),
            parent_folder_id: Set(new.parent_folder_id.map(|id| id.0)),
            name: Set(new.name),
            created_by_user_id: Set(created_by_user_id),
            created_by_api_key_id: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(folder_from)
            .map_err(db_err)
    }

    async fn find(&self, ctx: &WorkspaceCtx, id: FolderId) -> Result<Option<Folder>, DomainError> {
        folder::Entity::find_by_id(id.0)
            .filter(folder::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(folder::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(folder_from))
            .map_err(db_err)
    }

    async fn list_children(
        &self,
        ctx: &WorkspaceCtx,
        parent: Option<FolderId>,
    ) -> Result<Vec<Folder>, DomainError> {
        let mut q = folder::Entity::find()
            .filter(folder::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(folder::Column::DeletedAt.is_null());

        q = match parent {
            Some(pid) => q.filter(folder::Column::ParentFolderId.eq(pid.0)),
            None => q.filter(folder::Column::ParentFolderId.is_null()),
        };

        q.all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(folder_from).collect())
            .map_err(db_err)
    }

    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: FolderId,
        name: String,
    ) -> Result<(), DomainError> {
        let row = folder::Entity::find_by_id(id.0)
            .filter(folder::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(folder::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "folder",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.name = Set(name);
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: FolderId,
        new_parent: Option<FolderId>,
    ) -> Result<(), DomainError> {
        let row = folder::Entity::find_by_id(id.0)
            .filter(folder::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(folder::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "folder",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.parent_folder_id = Set(new_parent.map(|id| id.0));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: FolderId) -> Result<(), DomainError> {
        let row = folder::Entity::find_by_id(id.0)
            .filter(folder::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(folder::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "folder",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

fn user_id_from_actor(actor: &Actor) -> Option<uuid::Uuid> {
    match actor {
        Actor::User(uid) => Some(uid.0),
        Actor::ApiKey(_) => None,
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

fn internal_err(msg: String) -> DomainError {
    DomainError::Internal { message: msg }
}
