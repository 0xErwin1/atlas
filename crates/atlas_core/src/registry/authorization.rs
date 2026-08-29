use crate::ids::ActionId;

/// Authorization surface declared by a component (SHELL-REG-1).
pub struct Authorization {
    pub resource_kinds: Vec<String>,
    pub actions: Vec<ActionId>,
    pub role_definitions: Vec<String>,
    pub principal_sets: Vec<String>,
    pub provider: bool,
}
