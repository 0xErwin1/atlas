// B3: epic/task convention detection and mapping are implemented in Batch B3.
#![allow(dead_code)]

use super::parser::VaultDoc;
use super::plan::{BoardOp, LinkOp, TaskOp};

/// Inspects vault documents for the `type: epic|task` convention and returns
/// board, task, and link operations derived from that convention.
///
/// B3: Returns empty collections — the real mapping logic is added in Batch B3.
pub(crate) fn build_ops(_docs: &[VaultDoc]) -> (Vec<BoardOp>, Vec<TaskOp>, Vec<LinkOp>) {
    (vec![], vec![], vec![])
}
