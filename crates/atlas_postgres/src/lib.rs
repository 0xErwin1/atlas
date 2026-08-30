//! Neutral Postgres runtime plumbing: pool-sizing configuration and
//! connection construction.
//!
//! This crate owns exactly the parts of `atlas_server`'s database setup that
//! are product-neutral. It MUST NOT depend on `atlas_acta`, `atlas_custos`, or
//! any other product crate — `tests/dependency_boundary.rs` enforces that
//! invariant.

pub mod config;
pub mod connect;
pub mod error;

pub use config::{PoolConfig, PostgresConfig};
pub use connect::{connect, connect_options};
pub use error::db_err;
