//! Which functions can reach themselves.
//!
//! Vow has no loop syntax, so a call cycle is the only way a function can fail
//! to return. Finding them is Tarjan's algorithm and nothing more interesting
//! than that, written iteratively: the thing this exists to detect is unbounded
//! recursion, and a checker that blew its own stack looking for one would be
//! funny in the wrong way.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use vow_resolve::DefId;

/// Who calls whom, among the functions of one module.
pub type CallGraph = HashMap<DefId, Vec<DefId>>;

/// Every node that lies on a cycle.
///
/// That is a node whose strongly connected component has more than one member,
/// or one that calls itself directly. A component of one with no self edge is
/// an ordinary function and is not here.
///
/// Generic over the node because none of this is about definitions, and a
/// graph algorithm that can only be tested by building a resolution table is a
/// graph algorithm nobody tests.
pub fn on_a_cycle<T: Copy + Eq + Ord + Hash>(graph: &HashMap<T, Vec<T>>) -> HashSet<T> {
    let mut tarjan = Tarjan {
        graph,
        index: HashMap::new(),
        low: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        next: 0,
        found: HashSet::new(),
    };

    // Sorted so the walk does not depend on hash order. Nothing downstream
    // reads the order, but a pass whose behaviour changes between runs is a
    // pass nobody can debug.
    let mut roots: Vec<T> = graph.keys().copied().collect();
    roots.sort();

    for root in roots {
        if !tarjan.index.contains_key(&root) {
            tarjan.run(root);
        }
    }

    tarjan.found
}

struct Tarjan<'a, T> {
    graph: &'a HashMap<T, Vec<T>>,
    index: HashMap<T, u32>,
    low: HashMap<T, u32>,
    on_stack: HashSet<T>,
    stack: Vec<T>,
    next: u32,
    found: HashSet<T>,
}

impl<T: Copy + Eq + Ord + Hash> Tarjan<'_, T> {
    fn run(&mut self, root: T) {
        self.enter(root);
        // Each entry is a node and how many of its edges have been taken.
        let mut work: Vec<(T, usize)> = vec![(root, 0)];

        while let Some(&(node, taken)) = work.last() {
            let edges = self.edges(node);
            if taken < edges.len() {
                work.last_mut().expect("just looked at it").1 += 1;
                let next = edges[taken];
                if !self.index.contains_key(&next) {
                    self.enter(next);
                    work.push((next, 0));
                } else if self.on_stack.contains(&next) {
                    let seen = self.index[&next];
                    let low = self.low[&node].min(seen);
                    self.low.insert(node, low);
                }
                continue;
            }

            work.pop();
            if self.low[&node] == self.index[&node] {
                self.close(node);
            }
            if let Some(&(parent, _)) = work.last() {
                let low = self.low[&parent].min(self.low[&node]);
                self.low.insert(parent, low);
            }
        }
    }

    fn edges(&self, node: T) -> &[T] {
        self.graph.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }

    fn enter(&mut self, node: T) {
        self.index.insert(node, self.next);
        self.low.insert(node, self.next);
        self.next += 1;
        self.stack.push(node);
        self.on_stack.insert(node);
    }

    /// Pops one strongly connected component and records it if it is a cycle.
    fn close(&mut self, root: T) {
        let mut component = Vec::new();
        while let Some(node) = self.stack.pop() {
            self.on_stack.remove(&node);
            component.push(node);
            if node == root {
                break;
            }
        }

        let calls_itself = self.edges(root).contains(&root);
        if component.len() > 1 || calls_itself {
            self.found.extend(component);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_of(nodes: usize, edges: &[(u32, u32)]) -> HashMap<u32, Vec<u32>> {
        let mut graph: HashMap<u32, Vec<u32>> =
            (0..nodes as u32).map(|node| (node, Vec::new())).collect();
        for (from, to) in edges {
            graph.get_mut(from).expect("a node in the graph").push(*to);
        }
        graph
    }

    #[test]
    fn a_straight_line_has_no_cycle() {
        assert!(on_a_cycle(&graph_of(3, &[(0, 1), (1, 2)])).is_empty());
    }

    #[test]
    fn calling_yourself_is_a_cycle() {
        assert_eq!(
            on_a_cycle(&graph_of(2, &[(0, 0), (0, 1)])),
            HashSet::from([0])
        );
    }

    #[test]
    fn mutual_recursion_catches_both() {
        assert_eq!(
            on_a_cycle(&graph_of(2, &[(0, 1), (1, 0)])),
            HashSet::from([0, 1])
        );
    }

    #[test]
    fn a_caller_of_a_cycle_is_not_itself_on_one() {
        // It inherits `Diverge` through the row instead, which is the ordinary
        // propagation and not this function's business.
        assert_eq!(
            on_a_cycle(&graph_of(3, &[(0, 1), (1, 2), (2, 1)])),
            HashSet::from([1, 2])
        );
    }

    #[test]
    fn two_separate_cycles_are_both_found() {
        assert_eq!(
            on_a_cycle(&graph_of(5, &[(0, 1), (1, 0), (2, 3), (3, 2), (4, 0)])),
            HashSet::from([0, 1, 2, 3])
        );
    }

    #[test]
    fn a_long_chain_does_not_overflow() {
        // The whole point. A thousand nodes in a line, then one edge back to
        // the start, walked without a single recursive call.
        let mut edges: Vec<(u32, u32)> = (0..999).map(|index| (index, index + 1)).collect();
        edges.push((999, 0));

        assert_eq!(on_a_cycle(&graph_of(1000, &edges)).len(), 1000);
    }

    #[test]
    fn a_long_chain_with_no_cycle_finds_none() {
        let edges: Vec<(u32, u32)> = (0..999).map(|index| (index, index + 1)).collect();
        assert!(on_a_cycle(&graph_of(1000, &edges)).is_empty());
    }
}
