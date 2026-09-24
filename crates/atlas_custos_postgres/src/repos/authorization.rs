use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ActionId;
use atlas_custos::entities::authorization::{
    CustomRole, DenyRecord, DenyRuleId, GrantId, GrantRecord, NewCustomRole, NewDenyRecord,
    NewGrantRecord, ROLE_IN_USE_CONFLICT, RoleId, SubjectRecord, SubjectSet, TargetRecord,
};
use atlas_custos::ports::authorization::{DenyRuleRepo, GrantV2Repo, RoleRepo};
use chrono::Utc;
use sea_orm::error::SqlxError;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder, RuntimeErr,
};
use uuid::Uuid;

const ROLES_PRODUCT_NAME_KEY: &str = "custos_roles_product_name_key";
const GRANTS_V2_ROLE_ID_FKEY: &str = "custos_grants_v2_role_id_fkey";

/// The name of the Postgres constraint a database error was raised for,
/// when the error carries one. Classification by constraint name keeps the
/// typed mapping independent of the message wording.
fn violated_constraint(error: &DbErr) -> Option<&str> {
    let (DbErr::Exec(RuntimeErr::SqlxError(sqlx)) | DbErr::Query(RuntimeErr::SqlxError(sqlx))) =
        error
    else {
        return None;
    };

    match sqlx.as_ref() {
        SqlxError::Database(database) => database.constraint(),
        _ => None,
    }
}

use crate::entities::authorization::{
    actions_to_stored, authority_to_columns, deny_from, deny_rule, grant_from, grant_v2, role,
    role_from, subject_to_columns,
};
use atlas_postgres::db_err;

/// Rows addressed to any identity in `subjects`: the principal itself, one
/// of its groups, or one of its principal sets. Empty group or set lists
/// contribute no predicate.
fn subjects_condition<C: ColumnTrait>(
    principal: C,
    group: C,
    principal_set: C,
    subjects: &SubjectSet,
) -> Condition {
    let mut condition = Condition::any().add(principal.eq(subjects.principal.0));

    if !subjects.groups.is_empty() {
        let group_ids: Vec<Uuid> = subjects.groups.iter().map(|id| id.0).collect();
        condition = condition.add(group.is_in(group_ids));
    }

    if !subjects.principal_sets.is_empty() {
        let set_names: Vec<String> = subjects
            .principal_sets
            .iter()
            .map(ToString::to_string)
            .collect();
        condition = condition.add(principal_set.is_in(set_names));
    }

    condition
}

pub struct PgRoleRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl RoleRepo for PgRoleRepo {
    async fn create(&self, new: NewCustomRole) -> Result<CustomRole, DomainError> {
        Self::create_in(&self.conn, new).await
    }

    async fn get(&self, id: RoleId) -> Result<Option<CustomRole>, DomainError> {
        Self::get_in(&self.conn, id).await
    }

    async fn list_by_product(&self, product: &str) -> Result<Vec<CustomRole>, DomainError> {
        Self::list_by_product_in(&self.conn, product).await
    }

    async fn update(
        &self,
        id: RoleId,
        name: Option<String>,
        actions: Option<Vec<ActionId>>,
    ) -> Result<Option<CustomRole>, DomainError> {
        Self::update_in(&self.conn, id, name, actions).await
    }

    async fn delete(&self, id: RoleId) -> Result<bool, DomainError> {
        Self::delete_in(&self.conn, id).await
    }
}

impl PgRoleRepo {
    pub async fn create_in<C: ConnectionTrait>(
        conn: &C,
        new: NewCustomRole,
    ) -> Result<CustomRole, DomainError> {
        let now = Utc::now();
        let model = role::ActiveModel {
            id: Set(new.id.0),
            product: Set(new.product),
            name: Set(new.name),
            actions: Set(actions_to_stored(&new.actions)),
            created_by: Set(new.created_by.0),
            created_at: Set(now),
            updated_at: Set(now),
        };

        model
            .insert(conn)
            .await
            .map_err(|e| {
                if violated_constraint(&e) == Some(ROLES_PRODUCT_NAME_KEY) {
                    DomainError::AlreadyExists {
                        message: "a role with this name already exists in the product".into(),
                    }
                } else {
                    db_err(e)
                }
            })
            .and_then(role_from)
    }

    pub async fn get_in<C: ConnectionTrait>(
        conn: &C,
        id: RoleId,
    ) -> Result<Option<CustomRole>, DomainError> {
        role::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .map(role_from)
            .transpose()
    }

    pub async fn list_by_product_in<C: ConnectionTrait>(
        conn: &C,
        product: &str,
    ) -> Result<Vec<CustomRole>, DomainError> {
        role::Entity::find()
            .filter(role::Column::Product.eq(product))
            .order_by_asc(role::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(role_from)
            .collect()
    }

    /// Applies the requested field changes and bumps `updated_at`. Reads the
    /// row first so an unknown id answers `None` instead of a zero-row update
    /// indistinguishable from a no-op change.
    pub async fn update_in<C: ConnectionTrait>(
        conn: &C,
        id: RoleId,
        name: Option<String>,
        actions: Option<Vec<ActionId>>,
    ) -> Result<Option<CustomRole>, DomainError> {
        let Some(existing) = role::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
        else {
            return Ok(None);
        };

        let mut model: role::ActiveModel = existing.into();
        if let Some(name) = name {
            model.name = Set(name);
        }
        if let Some(actions) = actions {
            model.actions = Set(actions_to_stored(&actions));
        }
        model.updated_at = Set(Utc::now());

        model
            .update(conn)
            .await
            .map_err(|e| {
                if violated_constraint(&e) == Some(ROLES_PRODUCT_NAME_KEY) {
                    DomainError::AlreadyExists {
                        message: "a role with this name already exists in the product".into(),
                    }
                } else {
                    db_err(e)
                }
            })
            .and_then(role_from)
            .map(Some)
    }

    /// The custom role named `name` in `product`, if any.
    pub async fn find_by_name_in<C: ConnectionTrait>(
        conn: &C,
        product: &str,
        name: &str,
    ) -> Result<Option<CustomRole>, DomainError> {
        role::Entity::find()
            .filter(role::Column::Product.eq(product))
            .filter(role::Column::Name.eq(name))
            .one(conn)
            .await
            .map_err(db_err)?
            .map(role_from)
            .transpose()
    }

    /// The role named `new.name` in `new.product`, created from `new` when
    /// absent. A taken `(product, name)` is skipped with `ON CONFLICT DO
    /// NOTHING` rather than raised, so a creation that loses a race leaves
    /// the caller's transaction usable and returns the stored row, whose
    /// actions win over `new.actions`.
    pub async fn find_or_create_in<C: ConnectionTrait>(
        conn: &C,
        new: NewCustomRole,
    ) -> Result<CustomRole, DomainError> {
        let product = new.product.clone();
        let name = new.name.clone();
        let now = Utc::now();
        let model = role::ActiveModel {
            id: Set(new.id.0),
            product: Set(new.product),
            name: Set(new.name),
            actions: Set(actions_to_stored(&new.actions)),
            created_by: Set(new.created_by.0),
            created_at: Set(now),
            updated_at: Set(now),
        };

        role::Entity::insert(model)
            .on_conflict(
                OnConflict::columns([role::Column::Product, role::Column::Name])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(conn)
            .await
            .map_err(db_err)?;

        Self::find_by_name_in(conn, &product, &name)
            .await?
            .ok_or_else(|| DomainError::Internal {
                message: format!("role `{name}` of `{product}` is missing after its upsert"),
            })
    }

    /// Deletes a role by id. The `ON DELETE RESTRICT` foreign key from
    /// `custos.grants_v2.role_id` makes the delete fail while any grant
    /// still references the role; that failure surfaces as
    /// `ComponentConflict { code: ROLE_IN_USE_CONFLICT, .. }`.
    pub async fn delete_in<C: ConnectionTrait>(conn: &C, id: RoleId) -> Result<bool, DomainError> {
        let result = role::Entity::delete_by_id(id.0)
            .exec(conn)
            .await
            .map_err(|e| {
                if violated_constraint(&e) == Some(GRANTS_V2_ROLE_ID_FKEY) {
                    DomainError::ComponentConflict {
                        code: ROLE_IN_USE_CONFLICT,
                        message: Some("the role is still referenced by a grant".into()),
                    }
                } else {
                    db_err(e)
                }
            })?;

        Ok(result.rows_affected > 0)
    }
}

pub struct PgGrantV2Repo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl GrantV2Repo for PgGrantV2Repo {
    async fn create(&self, new: NewGrantRecord) -> Result<GrantRecord, DomainError> {
        Self::create_in(&self.conn, new).await
    }

    async fn get(&self, id: GrantId) -> Result<Option<GrantRecord>, DomainError> {
        Self::get_in(&self.conn, id).await
    }

    async fn list_by_product(&self, product: &str) -> Result<Vec<GrantRecord>, DomainError> {
        Self::list_by_product_in(&self.conn, product).await
    }

    async fn delete(&self, id: GrantId) -> Result<bool, DomainError> {
        Self::delete_in(&self.conn, id).await
    }

    async fn list_for_subjects(
        &self,
        product: &str,
        subjects: &SubjectSet,
    ) -> Result<Vec<GrantRecord>, DomainError> {
        Self::list_for_subjects_in(&self.conn, product, subjects).await
    }
}

impl PgGrantV2Repo {
    pub async fn create_in<C: ConnectionTrait>(
        conn: &C,
        new: NewGrantRecord,
    ) -> Result<GrantRecord, DomainError> {
        let (subject_kind, principal_id, group_id, principal_set) =
            subject_to_columns(&new.subject);
        let (authority_kind, role_name, role_version, role_id, actions) =
            authority_to_columns(&new.authority)?;

        let model = grant_v2::ActiveModel {
            id: Set(new.id.0),
            subject_kind: Set(subject_kind.to_string()),
            subject_principal_id: Set(principal_id),
            subject_group_id: Set(group_id),
            subject_principal_set: Set(principal_set),
            target_kind: Set(new.target.kind().as_str().to_string()),
            target: Set(new.target.canonical()),
            product: Set(new.target.product().to_string()),
            authority_kind: Set(authority_kind.to_string()),
            role_name: Set(role_name),
            role_version: Set(role_version),
            role_id: Set(role_id),
            actions: Set(actions),
            created_by: Set(new.created_by.0),
            created_at: Set(Utc::now()),
        };

        model
            .insert(conn)
            .await
            .map_err(db_err)
            .and_then(grant_from)
    }

    pub async fn get_in<C: ConnectionTrait>(
        conn: &C,
        id: GrantId,
    ) -> Result<Option<GrantRecord>, DomainError> {
        grant_v2::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .map(grant_from)
            .transpose()
    }

    pub async fn list_by_product_in<C: ConnectionTrait>(
        conn: &C,
        product: &str,
    ) -> Result<Vec<GrantRecord>, DomainError> {
        grant_v2::Entity::find()
            .filter(grant_v2::Column::Product.eq(product))
            .order_by_asc(grant_v2::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(grant_from)
            .collect()
    }

    pub async fn delete_in<C: ConnectionTrait>(conn: &C, id: GrantId) -> Result<bool, DomainError> {
        let result = grant_v2::Entity::delete_by_id(id.0)
            .exec(conn)
            .await
            .map_err(db_err)?;

        Ok(result.rows_affected > 0)
    }

    /// Every grant addressed to exactly `subject` on exactly `target`,
    /// whatever its authority, oldest id first.
    pub async fn find_matching_in<C: ConnectionTrait>(
        conn: &C,
        subject: &SubjectRecord,
        target: &TargetRecord,
    ) -> Result<Vec<GrantRecord>, DomainError> {
        let (subject_kind, principal_id, group_id, principal_set) = subject_to_columns(subject);

        // `eq(None)` renders `= NULL`, which matches nothing: the absent
        // subject columns must be matched with `IS NULL`.
        fn nullable<C: ColumnTrait, V: Into<sea_orm::Value>>(
            column: C,
            value: Option<V>,
        ) -> sea_orm::sea_query::SimpleExpr {
            match value {
                Some(value) => column.eq(value),
                None => column.is_null(),
            }
        }

        grant_v2::Entity::find()
            .filter(grant_v2::Column::SubjectKind.eq(subject_kind.to_string()))
            .filter(nullable(grant_v2::Column::SubjectPrincipalId, principal_id))
            .filter(nullable(grant_v2::Column::SubjectGroupId, group_id))
            .filter(nullable(
                grant_v2::Column::SubjectPrincipalSet,
                principal_set,
            ))
            .filter(grant_v2::Column::TargetKind.eq(target.kind().as_str().to_string()))
            .filter(grant_v2::Column::Target.eq(target.canonical()))
            .order_by_asc(grant_v2::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(grant_from)
            .collect()
    }

    pub async fn list_for_subjects_in<C: ConnectionTrait>(
        conn: &C,
        product: &str,
        subjects: &SubjectSet,
    ) -> Result<Vec<GrantRecord>, DomainError> {
        grant_v2::Entity::find()
            .filter(grant_v2::Column::Product.eq(product))
            .filter(subjects_condition(
                grant_v2::Column::SubjectPrincipalId,
                grant_v2::Column::SubjectGroupId,
                grant_v2::Column::SubjectPrincipalSet,
                subjects,
            ))
            .order_by_asc(grant_v2::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(grant_from)
            .collect()
    }
}

pub struct PgDenyRuleRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl DenyRuleRepo for PgDenyRuleRepo {
    async fn create(&self, new: NewDenyRecord) -> Result<DenyRecord, DomainError> {
        Self::create_in(&self.conn, new).await
    }

    async fn get(&self, id: DenyRuleId) -> Result<Option<DenyRecord>, DomainError> {
        Self::get_in(&self.conn, id).await
    }

    async fn list_by_product(&self, product: &str) -> Result<Vec<DenyRecord>, DomainError> {
        Self::list_by_product_in(&self.conn, product).await
    }

    async fn delete(&self, id: DenyRuleId) -> Result<bool, DomainError> {
        Self::delete_in(&self.conn, id).await
    }

    async fn list_for_subjects(
        &self,
        product: &str,
        subjects: &SubjectSet,
    ) -> Result<Vec<DenyRecord>, DomainError> {
        Self::list_for_subjects_in(&self.conn, product, subjects).await
    }
}

impl PgDenyRuleRepo {
    pub async fn create_in<C: ConnectionTrait>(
        conn: &C,
        new: NewDenyRecord,
    ) -> Result<DenyRecord, DomainError> {
        let (subject_kind, principal_id, group_id, principal_set) =
            subject_to_columns(&new.subject);

        let model = deny_rule::ActiveModel {
            id: Set(new.id.0),
            subject_kind: Set(subject_kind.to_string()),
            subject_principal_id: Set(principal_id),
            subject_group_id: Set(group_id),
            subject_principal_set: Set(principal_set),
            target_kind: Set(new.target.kind().as_str().to_string()),
            target: Set(new.target.canonical()),
            product: Set(new.target.product().to_string()),
            actions: Set(actions_to_stored(&new.actions)),
            created_by: Set(new.created_by.0),
            created_at: Set(Utc::now()),
        };

        model.insert(conn).await.map_err(db_err).and_then(deny_from)
    }

    pub async fn get_in<C: ConnectionTrait>(
        conn: &C,
        id: DenyRuleId,
    ) -> Result<Option<DenyRecord>, DomainError> {
        deny_rule::Entity::find_by_id(id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .map(deny_from)
            .transpose()
    }

    pub async fn list_by_product_in<C: ConnectionTrait>(
        conn: &C,
        product: &str,
    ) -> Result<Vec<DenyRecord>, DomainError> {
        deny_rule::Entity::find()
            .filter(deny_rule::Column::Product.eq(product))
            .order_by_asc(deny_rule::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(deny_from)
            .collect()
    }

    pub async fn delete_in<C: ConnectionTrait>(
        conn: &C,
        id: DenyRuleId,
    ) -> Result<bool, DomainError> {
        let result = deny_rule::Entity::delete_by_id(id.0)
            .exec(conn)
            .await
            .map_err(db_err)?;

        Ok(result.rows_affected > 0)
    }

    pub async fn list_for_subjects_in<C: ConnectionTrait>(
        conn: &C,
        product: &str,
        subjects: &SubjectSet,
    ) -> Result<Vec<DenyRecord>, DomainError> {
        deny_rule::Entity::find()
            .filter(deny_rule::Column::Product.eq(product))
            .filter(subjects_condition(
                deny_rule::Column::SubjectPrincipalId,
                deny_rule::Column::SubjectGroupId,
                deny_rule::Column::SubjectPrincipalSet,
                subjects,
            ))
            .order_by_asc(deny_rule::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(deny_from)
            .collect()
    }
}
