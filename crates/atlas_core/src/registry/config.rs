/// Typed configuration surface declared by a component (S3 `ATLAS_<COMPONENT>_`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDeclaration {
    struct_name: String,
    env_prefix: String,
    mandatory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigDeclarationError {
    #[error("config struct name must not be empty")]
    EmptyStructName,
    #[error("config env prefix must not be empty")]
    EmptyEnvPrefix,
    #[error("config env prefix contains illegal character `{ch}`")]
    InvalidEnvPrefixCharacter { ch: char },
    #[error("config env prefix must end with `_`")]
    MissingTrailingUnderscore,
}

impl ConfigDeclaration {
    pub fn new(
        struct_name: impl Into<String>,
        env_prefix: impl Into<String>,
        mandatory: bool,
    ) -> Result<Self, ConfigDeclarationError> {
        let struct_name = struct_name.into();
        let env_prefix = env_prefix.into();

        if struct_name.is_empty() {
            return Err(ConfigDeclarationError::EmptyStructName);
        }

        if env_prefix.is_empty() {
            return Err(ConfigDeclarationError::EmptyEnvPrefix);
        }

        if let Some(ch) = env_prefix
            .chars()
            .find(|ch| !ch.is_ascii_uppercase() && !ch.is_ascii_digit() && *ch != '_')
        {
            return Err(ConfigDeclarationError::InvalidEnvPrefixCharacter { ch });
        }

        if !env_prefix.ends_with('_') {
            return Err(ConfigDeclarationError::MissingTrailingUnderscore);
        }

        Ok(Self {
            struct_name,
            env_prefix,
            mandatory,
        })
    }

    pub fn struct_name(&self) -> &str {
        &self.struct_name
    }

    pub fn env_prefix(&self) -> &str {
        &self.env_prefix
    }

    pub fn mandatory(&self) -> bool {
        self.mandatory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_well_formed_prefix() {
        let config = ConfigDeclaration::new("ActaConfig", "ATLAS_ACTA_", true)
            .expect("valid config declaration");
        assert_eq!(config.struct_name(), "ActaConfig");
        assert_eq!(config.env_prefix(), "ATLAS_ACTA_");
        assert!(config.mandatory());
    }

    #[test]
    fn rejects_empty_struct_name() {
        assert_eq!(
            ConfigDeclaration::new("", "ATLAS_ACTA_", true),
            Err(ConfigDeclarationError::EmptyStructName)
        );
    }

    #[test]
    fn rejects_empty_env_prefix() {
        assert_eq!(
            ConfigDeclaration::new("ActaConfig", "", true),
            Err(ConfigDeclarationError::EmptyEnvPrefix)
        );
    }

    #[test]
    fn rejects_lowercase_env_prefix_character() {
        assert_eq!(
            ConfigDeclaration::new("ActaConfig", "atlas_", true),
            Err(ConfigDeclarationError::InvalidEnvPrefixCharacter { ch: 'a' })
        );
    }

    #[test]
    fn rejects_missing_trailing_underscore() {
        assert_eq!(
            ConfigDeclaration::new("ActaConfig", "ATLAS_ACTA", true),
            Err(ConfigDeclarationError::MissingTrailingUnderscore)
        );
    }
}
