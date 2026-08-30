//! Postgres persistence for Custos: sea-orm entities and repository
//! implementations for identity, group/permission grants, and the security
//! audit log.
//!
//! This crate depends only on `atlas_core`, `atlas_custos`, `atlas_postgres`
//! and sea-orm. It MUST NOT depend on `atlas_acta` as a cargo dependency, and
//! the target invariant is stronger: no SQL in this crate touches an
//! Acta-owned table. Orchestration that composes a Custos write with an Acta
//! write belongs in `atlas_server`, which owns both sides.
//!
//! The invariant holds: no SQL in this crate names an Acta-owned table. The
//! single api-key revoke path in `repos::identity` (`revoke_for_user_in`)
//! formerly issued a raw `DELETE FROM task_assignees` inside the revoke
//! transaction; that write moved to
//! `atlas_server::persistence::repos::boards_tasks::PgTaskAssigneeRepo::unassign_api_key_in`,
//! composed by the caller in the same transaction as the Custos-side
//! `revoke_for_user_in` update (design D4). The trait-level `revoke` and
//! `revoke_for_user` entry points, which could not compose that cleanup from
//! inside this crate, had no callers and were removed rather than left with
//! silently narrowed behavior. The
//! `dependency_boundary` test enforces the cargo edge, and
//! `no_acta_table_names_in_sql` (in that same test file) enforces the raw-SQL
//! half by scanning this crate's source for `task_assignees`, the one
//! Acta-owned table this crate ever touched; the full Acta table inventory is
//! covered by the S3d CI grep gate.

pub mod entities;
pub mod migrations;
pub mod repos;
