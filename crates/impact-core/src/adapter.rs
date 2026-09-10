use std::path::Path;

use serde::{Deserialize, Serialize};

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
    /// The last line of this symbol's own span (1-indexed, inclusive) — its closing
    /// brace/`end` for a block-bodied declaration, or the same as `line` for a
    /// single-line one. Lets `compute_diff_impact` map a touched line to the symbol that
    /// actually contains it instead of only the nearest preceding declaration.
    pub end_line: usize,
    pub is_test: bool,
    /// Whether this symbol was declared in a file the adapter recognized as
    /// tool-generated (Go's standard `// Code generated ... DO NOT EDIT.` marker, so
    /// far — see `impact-lang-go`). Never a guess: an adapter that can't structurally
    /// tell always leaves this `false`. Drives `Resolver::in_module` preferring a
    /// non-generated candidate when a package-scoped call resolves to more than one
    /// same-named method — a generated mock living beside its real implementation
    /// shouldn't be what silently downgrades that implementation's own confidence.
    pub is_generated: bool,
}

/// How far an adapter could narrow down what a reference's `to_name` actually refers to,
/// using only what's visible in the one file it parsed. The linker needs this because a
/// bare name on its own is very weak evidence: `prune()` matching exactly one `prune` in
/// the project does *not* mean the call goes there, and reporting that match as `Exact`
/// is worse than reporting nothing — it looks authoritative while being wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefTarget {
    /// The adapter resolved the name to a module: an import/`use` brought it in, it's
    /// declared in this same file, or it's visible from the same package. The string is a
    /// `::`-joined segment path; the linker matches symbols whose own qualified path
    /// contains those segments (see `Resolver::in_module`), which is what lets a
    /// directory-scoped language (Go, Kotlin) and a file-scoped one (TypeScript, Rust)
    /// share one rule.
    Module(String),
    /// The adapter looked for scope evidence and found none: a method call on a receiver
    /// whose type it can't determine, or a bare name with no matching import and no
    /// same-file declaration. Still resolved structurally — over-reporting beats missing
    /// a caller — but never as `Exact`.
    Opaque,
    /// The language has no file-level import that could narrow this name in the first
    /// place — Swift's whole-module namespace, where every top-level symbol is visible
    /// everywhere without ceremony. Structural resolution is then the best evidence
    /// available rather than a guess, so `Exact` stays on the table.
    Unscoped,
}

/// A reference found in one file, before the linker resolves it into a graph `Edge`.
/// `from_qualified_path` is the containing symbol (e.g. the function a call site is in);
/// `to_name` is the adapter's best-effort name for the target — a bare identifier, not
/// necessarily a full qualified path, since resolving it properly against the *project*
/// is the linker's job. `to_target` is what the adapter could work out about the name
/// from its own file: which module an import binds it to, or that it couldn't tell.
#[derive(Debug, Clone)]
pub struct RefDecl {
    pub from_qualified_path: String,
    pub to_name: String,
    pub kind: EdgeKind,
    pub to_target: RefTarget,
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
