use super::{CapabilityId, ComponentId, ContractVersion, ContractVersionRange, SatelliteMode};

/// A satellite integration declared by a component (SHELL-INT-3).
#[derive(Debug, Clone, PartialEq)]
pub struct SatelliteDeclaration {
    owner: ComponentId,
    capabilities: Vec<CapabilityId>,
    protocol_version: ContractVersion,
    compatible_range: ContractVersionRange,
    mode: SatelliteMode,
    negotiation: String,
    health: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SatelliteDeclarationError {
    #[error("satellite negotiation must not be empty")]
    EmptyNegotiation,
    #[error("satellite health must not be empty")]
    EmptyHealth,
}

impl SatelliteDeclaration {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: ComponentId,
        capabilities: Vec<CapabilityId>,
        protocol_version: ContractVersion,
        compatible_range: ContractVersionRange,
        mode: SatelliteMode,
        negotiation: impl Into<String>,
        health: impl Into<String>,
    ) -> Result<Self, SatelliteDeclarationError> {
        let negotiation = negotiation.into();
        let health = health.into();

        if negotiation.is_empty() {
            return Err(SatelliteDeclarationError::EmptyNegotiation);
        }

        if health.is_empty() {
            return Err(SatelliteDeclarationError::EmptyHealth);
        }

        Ok(Self {
            owner,
            capabilities,
            protocol_version,
            compatible_range,
            mode,
            negotiation,
            health,
        })
    }

    pub fn owner(&self) -> &ComponentId {
        &self.owner
    }

    pub fn capabilities(&self) -> &[CapabilityId] {
        &self.capabilities
    }

    pub fn protocol_version(&self) -> ContractVersion {
        self.protocol_version
    }

    pub fn compatible_range(&self) -> ContractVersionRange {
        self.compatible_range
    }

    pub fn mode(&self) -> SatelliteMode {
        self.mode
    }

    pub fn negotiation(&self) -> &str {
        &self.negotiation
    }

    pub fn health(&self) -> &str {
        &self.health
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_declaration() -> SatelliteDeclaration {
        SatelliteDeclaration::new(
            ComponentId::new("platform").expect("valid component id"),
            vec![CapabilityId::new("storage.blob").expect("valid capability id")],
            ContractVersion::new(2),
            ContractVersionRange::new(ContractVersion::new(1), ContractVersion::new(3))
                .expect("valid range"),
            SatelliteMode::Local,
            "handshake-v1",
            "health-v1",
        )
        .expect("valid satellite declaration")
    }

    #[test]
    fn accessors_expose_every_field() {
        let declaration = valid_declaration();

        assert_eq!(declaration.owner().as_str(), "platform");
        assert_eq!(declaration.capabilities().len(), 1);
        assert_eq!(declaration.protocol_version(), ContractVersion::new(2));
        assert!(
            declaration
                .compatible_range()
                .contains(ContractVersion::new(2))
        );
        assert_eq!(declaration.mode(), SatelliteMode::Local);
        assert_eq!(declaration.negotiation(), "handshake-v1");
        assert_eq!(declaration.health(), "health-v1");
    }

    #[test]
    fn rejects_empty_negotiation() {
        let result = SatelliteDeclaration::new(
            ComponentId::new("platform").expect("valid component id"),
            vec![],
            ContractVersion::new(1),
            ContractVersionRange::new(ContractVersion::new(1), ContractVersion::new(1))
                .expect("valid range"),
            SatelliteMode::Remote,
            "",
            "health-v1",
        );

        assert_eq!(result, Err(SatelliteDeclarationError::EmptyNegotiation));
    }

    #[test]
    fn rejects_empty_health() {
        let result = SatelliteDeclaration::new(
            ComponentId::new("platform").expect("valid component id"),
            vec![],
            ContractVersion::new(1),
            ContractVersionRange::new(ContractVersion::new(1), ContractVersion::new(1))
                .expect("valid range"),
            SatelliteMode::Remote,
            "handshake-v1",
            "",
        );

        assert_eq!(result, Err(SatelliteDeclarationError::EmptyHealth));
    }
}
