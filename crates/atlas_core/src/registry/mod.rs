//! Turns a `Vec<ComponentEntry>` into a validated, frozen `Registry` via
//! [`build()`], implementing SHELL-REG-3, SHELL-REG-5, SHELL-CAP-2, and the
//! section-5 validation matrix rows in scope for D2.
//!
//! The following section-5 matrix rows are **not** implemented here and are
//! deferred to a later epic:
//!
//! - "config mandatory presente y válida" (env-value config validation at
//!   process startup) — owned by `atlas_server`.
//! - "router = registry" and "ningún path/schema/tag/nav de Product ausente"
//!   (deriving the HTTP router and OpenAPI document from the registry, and
//!   Product surface coverage) — owned by V2-E3.
//! - "CLI/MCP cubren OpenAPI" (CLI and MCP surfaces covering the derived
//!   OpenAPI document) — owned by V2-E11.
//! - "capability optional: superficies ocultas si cero" (the snapshot half:
//!   hiding a surface when its optional capability has zero providers) —
//!   owned by V2-E3.

mod authorization;
mod build;
mod capability_id;
mod component_id;
mod component_kind;
mod config;
mod contract_version;
mod entry;
mod error;
mod graph;
mod name;
mod persistence;
mod route;
mod satellite;
mod satellite_mode;
mod schema_contract_id;
mod schema_id;
mod validated;
mod worker;

pub use authorization::Authorization;
pub use build::build;
pub use capability_id::CapabilityId;
pub use component_id::ComponentId;
pub use component_kind::{ComponentKind, ComponentKindParseError};
pub use config::{ConfigDeclaration, ConfigDeclarationError};
pub use contract_version::{ContractVersion, ContractVersionRange, ContractVersionRangeError};
pub use entry::{Api, Capabilities, ComponentEntry, Dependency, Diagnostics, Experience, Identity};
pub use error::RegistryBuildError;
pub use name::RegistryIdError;
pub use persistence::Persistence;
pub use route::{HttpMethod, HttpMethodParseError, RouteDeclaration, RoutePath, RoutePathError};
pub use satellite::{SatelliteDeclaration, SatelliteDeclarationError};
pub use satellite_mode::{SatelliteMode, SatelliteModeParseError};
pub use schema_contract_id::SchemaContractId;
pub use schema_id::SchemaId;
pub use validated::Registry;
pub use worker::{BoundWorkers, Worker, WorkerBindError, WorkerDeclaration, WorkerId};
