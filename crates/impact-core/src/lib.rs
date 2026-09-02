pub mod adapter;
pub mod cache;
pub mod change;
pub mod config;
pub mod diff;
pub mod engine;
pub mod graph;
pub mod indexer;
pub mod linker;
pub mod workspace;

pub use adapter::{ContractRef, ContractRole, FileAst, LanguageAdapter, RefDecl, SymbolDecl};
pub use cache::Cache;
pub use change::{parse_change, ChangeSpec, ParseChangeError};
pub use config::{DetectorConfig, EventStrategy};
pub use diff::{parse_unified_diff, DiffTouches};
pub use engine::{
    apply_explain, compute_change_impact, compute_diff_impact, compute_file_impact,
    compute_symbol_impact, filter_min_confidence, Dependent, ImpactReport,
};
pub use graph::{Confidence, ContractKind, Edge, EdgeKind, Node, NodeId, NodeKind, SymbolGraph};
pub use indexer::{IndexStats, Indexer};
pub use linker::{link, Resolver};
pub use workspace::{
    cross_project_matches, CrossProjectMatch, LinkConfidence, Workspace, WorkspaceImpactReport,
    WorkspaceLink, WorkspaceProject,
};
