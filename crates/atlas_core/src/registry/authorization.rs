use crate::ids::ActionId;

/// Authorization surface declared by a component (SHELL-REG-1).
#[derive(Debug)]
pub struct Authorization {
    pub resource_kinds: Vec<String>,
    pub actions: Vec<ActionId>,
    /// The built-in role names, kept for the SHELL-REG-4 cross-check.
    pub role_definitions: Vec<String>,
    /// The versioned built-in roles with their actions (V2). Every name here
    /// also appears in `role_definitions`.
    pub role_definitions_v2: Vec<RoleDeclaration>,
    pub principal_sets: Vec<String>,
    pub provider: bool,
}

/// One versioned built-in role a component declares: its actions may span
/// the component's own product and the Custos delegation actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDeclaration {
    pub name: String,
    pub version: u32,
    pub actions: Vec<ActionId>,
}
