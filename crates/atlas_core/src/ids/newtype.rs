/// Defines a UUIDv7-backed newtype id type with the common trait
/// implementations shared by every product id (`Display`, `Default`,
/// `From<Uuid>`, ordering, hashing, and transparent serde).
///
/// The macro body uses fully qualified paths (`::uuid::Uuid`,
/// `::serde::Serialize`, `::serde::Deserialize`) so it expands correctly
/// regardless of what the call site has in scope.
#[macro_export]
macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        #[serde(transparent)]
        #[serde(crate = "::serde")]
        pub struct $name(pub ::uuid::Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(::uuid::Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<::uuid::Uuid> for $name {
            fn from(id: ::uuid::Uuid) -> Self {
                Self(id)
            }
        }
    };
}

#[cfg(test)]
#[allow(
    unreachable_pub,
    reason = "id type exists only to exercise the macro in this test module"
)]
mod hygiene {
    // Deliberately imports neither `serde` nor `uuid` — the macro must expand
    // cleanly without either in scope at the call site.
    crate::define_id!(HygieneTestId);

    #[test]
    fn macro_expands_without_serde_or_uuid_imports_in_scope() {
        let a = HygieneTestId::new();
        let b = HygieneTestId::from(::uuid::Uuid::now_v7());
        assert_ne!(a, b, "distinct v7 uuids must produce distinct ids");

        let json = ::serde_json::to_string(&a).expect("serialize");
        assert_eq!(json, format!("\"{}\"", a.0));
    }
}
