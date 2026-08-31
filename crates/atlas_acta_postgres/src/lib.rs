#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Postgres persistence for Acta: sea-orm entities, repository
//! implementations, and schema migrations for Acta-owned tables, moved out of
//! `atlas_server::persistence` (mirrors `atlas_custos_postgres`).
//!
//! This crate depends only on `atlas_core`, `atlas_acta`, `atlas_postgres`
//! and sea-orm. It MUST NOT depend on `atlas_custos`, `atlas_api`, or
//! `atlas_server`.
//!
//! Entities, repos, and migrations move here incrementally across the S4
//! slice's PR chain. `platform.*` (`user_ui_state`/`ui_state`) does not get
//! its own crate: its entity and repo stay in `atlas_server` (design D2/D4),
//! but its `SET SCHEMA`/rename migration is authored here (D3), composed
//! into `acta_new()` ahead of every Acta `SET SCHEMA` batch.

pub mod entities;
pub mod live_ancestors;
pub mod migrations;
pub mod repos;
