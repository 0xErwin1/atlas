//! Compile-time authorization-scope extraction (design D5, `v2-e3-s2-router-audit`).
//!
//! `component_routes!` needs to read the `Capability` a handler actually
//! enforces without adding a second, hand-typed marker at the route
//! declaration call site — the whole point is that the table entry and the
//! handler's real signature are "the same fact read twice," not two facts a
//! human keeps in sync.
//!
//! `Authorized<R, M, S>` already carries `S: RequiredScope` as the (almost
//! always first) handler parameter, and `S::CAPABILITY` already exists
//! (`authz::authorized::RequiredScope`). `ExtractScope` reads that generic
//! parameter straight off a handler function item via a bounded blanket impl,
//! mirroring axum's own arity-polymorphic `impl_handler!` generator for
//! `Handler`: one impl per handler arity, distinguished by a marker tuple
//! type so the impls do not overlap. Calling `declared_scope` never invokes
//! the handler — it only unifies `S` from the handler's real signature and
//! returns the associated constant, which is why an `&self` receiver is
//! enough despite the `FnOnce` bound.

use super::authorized::{Authorized, MinRole, RequiredScope, ResolvedResource};
use atlas_custos::capability::Capability;

/// Reads the `Capability` (if any) a handler's `Authorized<R, M, S>`
/// parameter enforces, resolved by the compiler from the handler's real
/// signature. `Marker` disambiguates the per-arity blanket impls below;
/// callers never name it explicitly, they just call `.declared_scope()`.
pub trait ExtractScope<Marker> {
    fn declared_scope(&self) -> Option<Capability>;
}

/// Emits one blanket `ExtractScope` impl for a handler taking `Authorized<R,
/// M, S>` followed by the given extra parameters.
macro_rules! impl_extract_scope {
    ($($ty:ident),*) => {
        impl<F, Fut, R, M, S, $($ty,)*> ExtractScope<(R, M, S, $($ty,)*)> for F
        where
            F: FnOnce(Authorized<R, M, S>, $($ty,)*) -> Fut,
            R: ResolvedResource,
            M: MinRole,
            S: RequiredScope,
        {
            fn declared_scope(&self) -> Option<Capability> {
                S::CAPABILITY
            }
        }
    };
}

/// Recursively emits one `impl_extract_scope!` invocation per arity from the
/// full parameter list down to zero extra parameters, so `T1..=T15` yields
/// impls for every arity in `0..=15` — axum's own handler-arity ceiling
/// (`axum::handler::impl_handler!`). The codebase's observed real handler
/// arity tops out at 4 extra parameters after `Authorized<...>` (see
/// `extract_scope_tests::arity_four_with_non_default_scope`); the extra
/// headroom to 15 costs nothing but macro-expansion time.
macro_rules! extract_scope_for_arities {
    () => {
        impl_extract_scope!();
    };
    ($head:ident $(, $tail:ident)*) => {
        impl_extract_scope!($head $(, $tail)*);
        extract_scope_for_arities!($($tail),*);
    };
}

extract_scope_for_arities!(
    T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15
);

#[cfg(test)]
mod tests {
    //! Scratch handlers proving `ExtractScope` resolves the right
    //! `Capability` across several real arities (T1.9/T1.10), including one
    //! handler pinned to a non-default `S`. None of these handlers are ever
    //! called — `declared_scope` never invokes `self`, it only reads
    //! `S::CAPABILITY` through the blanket impl the compiler selected for
    //! this handler's concrete signature.
    #![allow(clippy::unused_async, reason = "handler shape only, never invoked")]

    use super::ExtractScope;
    use crate::authz::authorized::{Authorized, NoScope, TasksRead, ViewerMin, WorkspaceRes};
    use crate::state::AppState;
    use atlas_custos::capability::{Capability, CapabilityAction, CapabilityFamily};
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use serde_json::Value;
    use std::collections::HashMap;

    async fn arity_zero_no_scope(
        _auth: Authorized<WorkspaceRes, ViewerMin, NoScope>,
    ) -> StatusCode {
        StatusCode::OK
    }

    async fn arity_two_no_scope(
        _auth: Authorized<WorkspaceRes, ViewerMin, NoScope>,
        State(_state): State<AppState>,
        Query(_q): Query<HashMap<String, String>>,
    ) -> StatusCode {
        StatusCode::OK
    }

    async fn arity_four_with_non_default_scope(
        _auth: Authorized<WorkspaceRes, ViewerMin, TasksRead>,
        State(_state): State<AppState>,
        Path(_id): Path<String>,
        Query(_q): Query<HashMap<String, String>>,
        axum::Json(_body): axum::Json<Value>,
    ) -> StatusCode {
        StatusCode::OK
    }

    #[test]
    fn arity_zero_resolves_no_scope() {
        assert_eq!(arity_zero_no_scope.declared_scope(), None);
    }

    #[test]
    fn arity_two_resolves_no_scope() {
        assert_eq!(arity_two_no_scope.declared_scope(), None);
    }

    #[test]
    fn arity_four_resolves_the_pinned_non_default_scope() {
        assert_eq!(
            arity_four_with_non_default_scope.declared_scope(),
            Some(Capability {
                family: CapabilityFamily::Tasks,
                action: CapabilityAction::Read,
            })
        );
    }
}
