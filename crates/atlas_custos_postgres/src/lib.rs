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
//! Known temporary exception, carried over verbatim from the pre-split code:
//! the api-key revoke paths in `repos::identity` still issue a raw
//! `DELETE FROM task_assignees` inside the revoke transaction. The next
//! change in this series splits that write out to `atlas_server` composition;
//! until then, the `dependency_boundary` test enforces only the cargo edge,
//! not the raw-SQL half of the invariant.

pub mod entities;
pub mod migrations;
pub mod repos;
