use std::collections::{BTreeMap, BTreeSet};

use super::ComponentId;

/// Shared topological sort used by `validate_dependencies` (acyclicity only)
/// and `validate_migration_order` (order plus acyclicity).
///
/// `edges` maps a prerequisite to the nodes that must follow it. Edge
/// endpoints absent from `nodes` are ignored; callers report those
/// separately. On success, `nodes` not mentioned by any edge sort
/// lexicographically alongside the rest. On failure, `Err` carries the
/// normalized cycle chain (D-8): a deterministic DFS from the
/// lexicographically smallest unemitted node, visiting successors in
/// `BTreeSet` order, rotated to start at its lexicographically smallest
/// member, without repeating the first element.
pub(crate) fn topological_order(
    nodes: &BTreeSet<ComponentId>,
    edges: &BTreeMap<ComponentId, BTreeSet<ComponentId>>,
) -> Result<Vec<ComponentId>, Vec<ComponentId>> {
    let mut in_degree: BTreeMap<ComponentId, usize> =
        nodes.iter().map(|node| (node.clone(), 0)).collect();

    for (source, targets) in edges {
        if !nodes.contains(source) {
            continue;
        }

        for target in targets {
            if let Some(degree) = in_degree.get_mut(target) {
                *degree += 1;
            }
        }
    }

    let mut ready: BTreeSet<ComponentId> = in_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| node.clone())
        .collect();

    let mut order = Vec::with_capacity(nodes.len());

    while let Some(node) = ready.iter().next().cloned() {
        ready.remove(&node);
        order.push(node.clone());

        if let Some(targets) = edges.get(&node) {
            for target in targets {
                let Some(degree) = in_degree.get_mut(target) else {
                    continue;
                };

                *degree -= 1;
                if *degree == 0 {
                    ready.insert(target.clone());
                }
            }
        }
    }

    if order.len() == nodes.len() {
        return Ok(order);
    }

    Err(find_cycle(nodes, edges))
}

/// Runs a deterministic DFS from the lexicographically smallest node not yet
/// emitted by Kahn's algorithm, visiting successors in `BTreeSet` order,
/// until it re-enters a node already on the current stack. Returns the
/// discovered cycle rotated to start at its lexicographically smallest
/// member, without repeating the first element.
fn find_cycle(
    nodes: &BTreeSet<ComponentId>,
    edges: &BTreeMap<ComponentId, BTreeSet<ComponentId>>,
) -> Vec<ComponentId> {
    let mut visited: BTreeSet<ComponentId> = BTreeSet::new();

    for start in nodes {
        if visited.contains(start) {
            continue;
        }

        let mut stack: Vec<ComponentId> = Vec::new();
        if let Some(cycle) = visit(start, edges, nodes, &mut visited, &mut stack) {
            return normalize_cycle(cycle);
        }
    }

    Vec::new()
}

fn visit(
    node: &ComponentId,
    edges: &BTreeMap<ComponentId, BTreeSet<ComponentId>>,
    nodes: &BTreeSet<ComponentId>,
    visited: &mut BTreeSet<ComponentId>,
    stack: &mut Vec<ComponentId>,
) -> Option<Vec<ComponentId>> {
    if let Some(position) = stack.iter().position(|entry| entry == node) {
        return Some(stack.get(position..).unwrap_or_default().to_vec());
    }

    if visited.contains(node) {
        return None;
    }

    stack.push(node.clone());

    if let Some(targets) = edges.get(node) {
        for target in targets {
            if !nodes.contains(target) {
                continue;
            }

            if let Some(cycle) = visit(target, edges, nodes, visited, stack) {
                return Some(cycle);
            }
        }
    }

    stack.pop();
    visited.insert(node.clone());

    None
}

/// Rotates a cycle to start at its lexicographically smallest member.
fn normalize_cycle(cycle: Vec<ComponentId>) -> Vec<ComponentId> {
    let Some((min_position, _)) = cycle.iter().enumerate().min_by_key(|(_, node)| *node) else {
        return cycle;
    };

    let mut rotated = Vec::with_capacity(cycle.len());
    rotated.extend_from_slice(cycle.get(min_position..).unwrap_or_default());
    rotated.extend_from_slice(cycle.get(..min_position).unwrap_or_default());
    rotated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn nodes(values: &[&str]) -> BTreeSet<ComponentId> {
        values.iter().map(|value| component(value)).collect()
    }

    fn edges(pairs: &[(&str, &[&str])]) -> BTreeMap<ComponentId, BTreeSet<ComponentId>> {
        pairs
            .iter()
            .map(|(from, targets)| {
                (
                    component(from),
                    targets.iter().map(|value| component(value)).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn empty_input_returns_empty_order() {
        let result = topological_order(&BTreeSet::new(), &BTreeMap::new());
        assert_eq!(result, Ok(vec![]));
    }

    #[test]
    fn isolated_nodes_sort_lexicographically() {
        let result = topological_order(&nodes(&["c", "a", "b"]), &BTreeMap::new());
        assert_eq!(
            result,
            Ok(vec![component("a"), component("b"), component("c")])
        );
    }

    #[test]
    fn linear_chain_orders_prerequisite_first() {
        let result = topological_order(
            &nodes(&["a", "b", "c"]),
            &edges(&[("a", &["b"]), ("b", &["c"])]),
        );
        assert_eq!(
            result,
            Ok(vec![component("a"), component("b"), component("c")])
        );
    }

    #[test]
    fn diamond_orders_root_before_both_branches_before_sink() {
        let result = topological_order(
            &nodes(&["a", "b", "c", "d"]),
            &edges(&[("a", &["b", "c"]), ("b", &["d"]), ("c", &["d"])]),
        );
        assert_eq!(
            result,
            Ok(vec![
                component("a"),
                component("b"),
                component("c"),
                component("d")
            ])
        );
    }

    #[test]
    fn two_independent_roots_tie_break_lexicographically() {
        let result = topological_order(&nodes(&["b", "a"]), &BTreeMap::new());
        assert_eq!(result, Ok(vec![component("a"), component("b")]));
    }

    #[test]
    fn edge_endpoint_not_in_nodes_is_ignored() {
        let result = topological_order(&nodes(&["a"]), &edges(&[("a", &["ghost"])]));
        assert_eq!(result, Ok(vec![component("a")]));
    }

    #[test]
    fn self_loop_reports_single_node_cycle() {
        let result = topological_order(&nodes(&["a"]), &edges(&[("a", &["a"])]));
        assert_eq!(result, Err(vec![component("a")]));
    }

    #[test]
    fn two_cycle_reports_both_nodes() {
        let result =
            topological_order(&nodes(&["a", "b"]), &edges(&[("a", &["b"]), ("b", &["a"])]));
        assert_eq!(result, Err(vec![component("a"), component("b")]));
    }

    #[test]
    fn three_cycle_rotates_to_start_at_smallest_member() {
        let result = topological_order(
            &nodes(&["a", "b", "c"]),
            &edges(&[("b", &["c"]), ("c", &["a"]), ("a", &["b"])]),
        );
        assert_eq!(
            result,
            Err(vec![component("a"), component("b"), component("c")])
        );
    }

    #[test]
    fn cycle_plus_acyclic_tail_reports_only_cycle_members() {
        let result = topological_order(
            &nodes(&["a", "b", "c", "d"]),
            &edges(&[("a", &["b"]), ("b", &["a", "c"]), ("c", &["d"])]),
        );
        assert_eq!(result, Err(vec![component("a"), component("b")]));
    }

    #[test]
    fn repeated_calls_are_deterministic() {
        let node_set = nodes(&["a", "b", "c"]);
        let edge_set = edges(&[("a", &["b"]), ("b", &["c"])]);

        let first = topological_order(&node_set, &edge_set);
        let second = topological_order(&node_set, &edge_set);
        assert_eq!(first, second);
    }
}
