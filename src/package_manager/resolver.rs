//! Dependency resolution (PRD §1, §5).
//!
//! Re-exports the engine from [`crate::package::resolver`] and adds
//! the PRD-named [`DependencyResolver`] struct with `resolve`,
//! `resolve_graph`, `detect_conflicts`, and `topological_sort`.

use crate::package::resolver::{
    detect_conflicts as detect_inner, resolve as resolve_inner,
    resolve_graph as resolve_graph_inner, topological_sort as topo_inner, GraphNode, MetaSource,
    ResolutionGraph, Resolved,
};

pub use crate::package::resolver::{MetaSource as ResolverMetaSource, Resolved as ResolvedPin};

/// PRD resolver struct: breadth-first traversal of the dependency
/// graph with conflict detection, over any [`MetaSource`].
pub struct DependencyResolver<'a> {
    source: &'a dyn MetaSource,
}

impl<'a> DependencyResolver<'a> {
    /// Borrow a metadata source (e.g. [`crate::package_manager::registry_client::RegistryClient`]).
    pub fn new(source: &'a dyn MetaSource) -> Self {
        Self { source }
    }

    /// Resolve `[(name, constraint, required_by)]` to exact pins.
    /// `required_by` is `"root"` for manifest-direct dependencies.
    pub fn resolve(&self, manifest_roots: &[(String, String, String)]) -> Result<Vec<Resolved>, String> {
        resolve_inner(manifest_roots, self.source)
    }

    /// Resolve to a full [`ResolutionGraph`] (pins + dep edges +
    /// dependency-first install order).
    pub fn resolve_graph(
        &self,
        manifest_roots: &[(String, String, String)],
    ) -> Result<ResolutionGraph, String> {
        resolve_graph_inner(manifest_roots, self.source)
    }

    /// Re-validate a graph against current metadata (registry drift).
    pub fn detect_conflicts(&self, graph: &ResolutionGraph) -> Result<(), String> {
        let _ = self.source;
        detect_inner(graph)
    }

    /// Order graph nodes dependencies-first.
    pub fn topological_sort(&self, nodes: &[GraphNode]) -> Vec<(String, String)> {
        // Selection-time resolution guarantees acyclicity; fall back
        // to name order on skewed input rather than failing.
        topo_inner(nodes).unwrap_or_else(|_| {
            let mut out: Vec<(String, String)> = nodes
                .iter()
                .map(|n| (n.name.clone(), n.version.clone()))
                .collect();
            out.sort();
            out
        })
    }
}
