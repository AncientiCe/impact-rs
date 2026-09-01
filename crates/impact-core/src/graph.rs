use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractKind {
    ApiRoute,
    Event,
    Table,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    Module,
    Type,
    Function,
    Field,
    Trait,
    Contract(ContractKind),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    /// Stable id derived from where a symbol lives, not where it's stored — the same
    /// symbol re-indexed from an unchanged file always gets the same id.
    pub fn new(project_id: &str, qualified_path: &str, kind: NodeKind) -> Self {
        let seed = format!("{project_id}\u{0}{qualified_path}\u{0}{kind:?}");
        NodeId(blake3::hash(seed.as_bytes()).to_hex().to_string())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub qualified_path: String,
    pub file: String,
    pub line: usize,
    pub language: String,
    /// Whether this `Function` node is a test (`#[test]`, `#[tokio::test]`, ...). Always
    /// `false` for non-function kinds. Drives the TESTS section of a blast-radius report.
    pub is_test: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    Calls,
    References,
    Implements,
    Contains,
    Imports,
    Produces,
    Consumes,
    Reads,
    Writes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Exact,
    Probable,
    Heuristic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

/// The language-agnostic symbol graph a project indexes into. Language adapters never
/// touch this directly — they emit `SymbolDecl`s (see `adapter.rs`) that the indexer
/// turns into `Node`s here.
#[derive(Debug, Default)]
pub struct SymbolGraph {
    nodes: HashMap<NodeId, Node>,
    edges: Vec<Edge>,
}

impl SymbolGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_node(&mut self, node: Node) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn insert_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
