use std::path::Path;

use crate::graph::{ContractKind, EdgeKind, NodeKind};

/// A parsed source file: the tree-sitter tree plus what produced it. `impact-core`
/// depends on the generic `tree-sitter` crate (parsing infrastructure, language-agnostic)
/// but never on a specific grammar crate like `tree-sitter-rust` — that dependency lives
/// only in the adapter crate for that language.
pub struct FileAst {
    pub path: String,
    pub source: String,
    pub tree: tree_sitter::Tree,
}

/// A symbol found in one file, before it's turned into a graph `Node`. The indexer
/// (not the adapter) computes the symbol's stable `NodeId`, so adapters don't need to
/// know about project identity.
#[derive(Debug, Clone)]
pub struct SymbolDecl {
    pub kind: NodeKind,
    pub qualified_path: String,
    pub line: usize,
    pub is_test: bool,
}

/// A reference found in one file, before the linker resolves it into a graph `Edge`.
/// `from_qualified_path` is the containing symbol (e.g. the function a call site is in);
/// `to_name` is the adapter's best-effort name for the target — a bare identifier, not
/// necessarily a full qualified path, since resolving it properly (against imports,
/// types, scope) is the linker's job, not the adapter's.
#[derive(Debug, Clone)]
pub struct RefDecl {
    pub from_qualified_path: String,
    pub to_name: String,
    pub kind: EdgeKind,
}

/// Which side of a contract relationship a symbol is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractRole {
    Produces,
    Consumes,
    Reads,
    Writes,
}

/// A symbol's relationship to a contract (an API route, an event type, a database
/// table), before the linker resolves both ends into a graph `Edge`.
///
/// `symbol_name` is resolved the same way `RefDecl.to_name` is (exact qualified-path
/// match first, short-name fallback), so an adapter can pass either a fully-resolved
/// qualified path (e.g. the function it's currently inside, already known exactly) or a
/// bare name it can't resolve itself (e.g. an axum route's handler, referenced only by
/// name at the registration site) — the linker doesn't need to know which.
///
/// `contract_id` is always resolved as an exact match against a `Contract` node's
/// qualified path — never short-name fallback, since contract identities (`"POST
/// /payments"`, `"PaymentCreated"`, `"payments"`) are deliberately bare, project-wide-
/// unique strings, not scoped names that could collide with something unrelated.
#[derive(Debug, Clone)]
pub struct ContractRef {
    pub contract_kind: ContractKind,
    pub contract_id: String,
    pub symbol_name: String,
    pub role: ContractRole,
}

/// One language's plug-in to the indexer. Implementations own everything specific to
/// their language (grammar, symbol shapes); the core graph, cache, and query engine
/// never depend on a specific language.
pub trait LanguageAdapter: Send + Sync {
    fn language_id(&self) -> &'static str;

    /// Glob patterns (relative to a project root) this adapter claims, e.g. `**/*.rs`.
    fn file_globs(&self) -> &[&str];

    fn parse_file(&self, path: &Path, source: &str) -> anyhow::Result<FileAst>;

    fn extract_symbols(&self, ast: &FileAst) -> Vec<SymbolDecl>;

    fn extract_references(&self, ast: &FileAst) -> Vec<RefDecl>;

    /// API/event/database contract relationships found in this file. Framework-specific
    /// (which macros, which route-registration shape) and driven by whatever detector
    /// configuration the adapter was constructed with.
    fn extract_contract_refs(&self, ast: &FileAst) -> Vec<ContractRef>;
}
