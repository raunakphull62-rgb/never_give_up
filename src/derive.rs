//! Stage 5: inspectable derive/metaprogramming (foundation).
//!
//! Only typed, append-only `derive`-style declarations. Every generated node
//! carries an origin chain back to its source. No arbitrary proc macros.

use crate::ast::NodeId;

/// Where a derived node came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeriveOrigin {
    pub generated_from: NodeId,
    pub rule: String,
}

/// One derived declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedNode {
    pub id: NodeId,
    pub name: String,
    pub origin: DeriveOrigin,
}

/// Append-only registry: entries are only pushed, never mutated/removed.
#[derive(Debug, Clone, Default)]
pub struct DeriveRegistry {
    entries: Vec<DerivedNode>,
}

impl DeriveRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Expand a derive. The child id extends the parent scope path.
    pub fn expand(&mut self, from: &NodeId, rule: &str, name: &str, index: u32) -> &DerivedNode {
        let mut path = from.path.clone();
        path.push(index);
        self.entries.push(DerivedNode {
            id: NodeId::new(path),
            name: name.to_string(),
            origin: DeriveOrigin {
                generated_from: from.clone(),
                rule: rule.to_string(),
            },
        });
        self.entries.last().expect("just pushed")
    }

    pub fn entries(&self) -> &[DerivedNode] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
