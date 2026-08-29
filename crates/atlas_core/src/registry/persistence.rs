use super::{ComponentId, SchemaContractId, SchemaId};

/// Persistence surface owned by a component (SHELL-REG-1).
#[derive(Debug)]
pub struct Persistence {
    pub schema: SchemaId,
    pub migration_owner: ComponentId,
    pub schema_contracts_provided: Vec<SchemaContractId>,
    pub schema_contracts_required: Vec<SchemaContractId>,
}
