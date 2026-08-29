pub use atlas_core::error::{DomainError, RevisionConflict};

pub mod acta_conflict {
    /// Fractional position space in a column is exhausted: no midpoint can be
    /// computed between the two anchors. The adapter must rebalance the
    /// column's keys and retry, or surface a 409 to the caller.
    pub const POSITION_EXHAUSTED: &str = "position-exhausted";
}
