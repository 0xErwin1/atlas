//! Postgres persistence for Acta: sea-orm entities, repository
//! implementations, and schema migrations for Acta-owned tables, moved out of
//! `atlas_server::persistence` (mirrors `atlas_custos_postgres`).
//!
//! This crate depends only on `atlas_core`, `atlas_acta`, `atlas_postgres`
//! and sea-orm. It MUST NOT depend on `atlas_custos`, `atlas_api`, or
//! `atlas_server`.
//!
//! Entities, repos, and migrations move here incrementally across the S4
//! slice's PR chain; `platform.*` (`user_ui_state`) is out of scope and stays
//! in `atlas_server` (design D2/D4).

pub mod entities;
