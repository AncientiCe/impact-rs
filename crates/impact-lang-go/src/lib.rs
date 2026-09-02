//! A Go `LanguageAdapter`. Mirrors every other adapter's structure and scope boundaries
//! (see `impact-lang-ts`'s module doc): functions, types, and methods
//! (`extract_symbols`), calls (`extract_references`). Contract detection
//! (`extract_contract_refs`) is scoped to one API route shape — see below — with no
//! events/database detection, an honest empty result for those rather than a stub.
//!
//! Structurally different from the others in one way, confirmed via a real parse-tree
//! dump before writing this: Go has no impl-block/class-body nesting at all. A method is
//! its own *top-level* declaration (`method_declaration`, distinct from
//! `function_declaration`) carrying a receiver — `func (t T) Method()` — so its qualified
//! path is built directly from the receiver's type name rather than by recursing into an
//! enclosing block the way `impact-lang-rust`'s `impl` handling or `impact-lang-ts`'s
//! `class_declaration` handling do.
//!
//! Detects tests: the standard-library `go test` convention is a `TestXxx` function in a
//! `_test.go` file — not a third-party framework choice, so (like Python's `test`-prefix
//! convention, and unlike TypeScript's fragmented Jest/Vitest/Mocha situation) it's cheap
//! and unambiguous enough to wire up.
//!
//! Also detects one API contract shape (gated on `impact.toml`'s `api_frameworks`
//! containing `"net/http"`, on by default): `net/http`'s Go 1.22+ enhanced routing,
//! `mux.HandleFunc("METHOD /path", handler)` — a method-prefixed pattern string, which
//! happens to already be exactly the `"{VERB} {path}"` shape `impact-lang-rust`'s axum
//! detector produces, so a Go and a Rust service registering the same route are
//! identity-matchable across a workspace with no extra normalization. Older,
//! method-less patterns (`mux.HandleFunc("/path", handler)`) aren't recognized — there's
//! no verb to report, and guessing one would be exactly the kind of silent wrong answer
//! this tool exists to avoid.

use std::path::Path;

use impact_core::{
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, FileAst, LanguageAdapter,
    NodeKind, RefDecl, SymbolDecl,
};
use tree_sitter::Node;

const HTTP_VERBS: &[&str] = &[
    "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "CONNECT", "TRACE",
];

pub struct GoAdapter {
    config: DetectorConfig,
}

impl GoAdapter {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

    fn language() -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }
}

impl Default for GoAdapter {
    fn default() -> Self {
        Self::new(DetectorConfig::default())
    }
}

impl LanguageAdapter for GoAdapter {
    fn language_id(&self) -> &'static str {
        "go"
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn parse_file(&self, path: &Path, source: &str) -> anyhow::Result<FileAst> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&Self::language())?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse {}", path.display()))?;
        Ok(FileAst {
            path: path.to_string_lossy().replace('\\', "/"),
            source: source.to_string(),
            tree,
        })
    }

    fn extract_symbols(&self, ast: &FileAst) -> Vec<SymbolDecl> {
        let prefix = module_prefix(&ast.path);
        let is_test_file = ast.path.ends_with("_test.go");
        let mut out = Vec::new();
        walk(
            ast.tree.root_node(),
            ast.source.as_bytes(),
            &prefix,
            is_test_file,
            &mut out,
        );
        out
    }

    fn extract_references(&self, ast: &FileAst) -> Vec<RefDecl> {
        let prefix = module_prefix(&ast.path);
        let mut out = Vec::new();
        collect_refs(
            ast.tree.root_node(),
            ast.source.as_bytes(),
            &prefix,
            None,
            &mut out,
        );
        out
    }

    fn extract_contract_refs(&self, ast: &FileAst) -> Vec<ContractRef> {
        let mut out = Vec::new();
        collect_contracts(
            ast.tree.root_node(),
            ast.source.as_bytes(),
            &self.config,
            &mut out,
        );
        out
    }
}

/// Derives a module path from a file path relative to the project root, e.g.
/// `payment/service.go` -> `payment::service`. Notably more approximate for Go than for
/// the other adapters: real Go packages are per-*directory* (every `.go` file in one
/// directory shares a package and can call each other unqualified), not per-file — this
/// still produces correct blast-radius results for same-package calls, since a bare
/// unqualified call site resolves through the linker's short-name fallback tier exactly
/// like an unqualified call in any other language, but the qualified path shown is
/// file-based rather than the Go-idiomatic package-based name.
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let path = path.strip_suffix(".go").unwrap_or(&path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segments.join("::")
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

fn field_text<'a>(node: Node, field: &str, source: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name(field)?.utf8_text(source).ok()
}

fn push(
    out: &mut Vec<SymbolDecl>,
    kind: NodeKind,
    qualified_path: String,
    node: Node,
    is_test: bool,
) {
    out.push(SymbolDecl {
        kind,
        qualified_path,
        line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        is_test,
    });
}

/// The rightmost identifier-like leaf: `foo` for `foo()`, `T` for a `(t T)` or `(t *T)`
/// receiver. Same structural, non-type-aware approach as every other adapter's
/// `last_identifier_text`.
fn last_identifier_text<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if matches!(
        node.kind(),
        "identifier" | "field_identifier" | "type_identifier"
    ) {
        return node.utf8_text(source).ok();
    }
    let mut cursor = node.walk();
    let mut result = None;
    for child in node.children(&mut cursor) {
        if let Some(text) = last_identifier_text(child, source) {
            result = Some(text);
        }
    }
    result
}

/// A `TestXxx` function in a `_test.go` file, per the `go test` standard-library
/// convention — not a third-party framework attribute the way Kotlin/Swift testing is.
fn is_go_test(name: &str, is_test_file: bool) -> bool {
    is_test_file && name.starts_with("Test")
}

/// The type name a `type_declaration` declares — its first `type_spec` child's own first
/// `type_identifier` child. Go allows multiple comma-separated specs per declaration;
/// this only extracts the first, matching every other adapter's "one clear case, not
/// exhaustive grammar coverage" scope.
fn type_declaration_name<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    let type_spec = node
        .children(&mut cursor)
        .find(|c| c.kind() == "type_spec")?;
    let mut spec_cursor = type_spec.walk();
    let name_node = type_spec
        .children(&mut spec_cursor)
        .find(|c| c.kind() == "type_identifier")?;
    name_node.utf8_text(source).ok()
}

/// Walks top-level declarations, extracting one `SymbolDecl` per function, type, and
/// method. Doesn't descend into function bodies — nested declarations are out of scope,
/// matching every other adapter's `walk`. Unlike the others, there's no class-body
/// recursion case: Go methods are top-level declarations in their own right (see the
/// module doc), so `method_declaration` is handled directly, not via nesting.
fn walk(node: Node, source: &[u8], prefix: &str, is_test_file: bool, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(
                        out,
                        NodeKind::Function,
                        join_path(prefix, name),
                        child,
                        is_go_test(name, is_test_file),
                    );
                }
            }
            "method_declaration" => {
                if let (Some(receiver), Some(name)) = (
                    child.child_by_field_name("receiver"),
                    field_text(child, "name", source),
                ) {
                    if let Some(receiver_type) = last_identifier_text(receiver, source) {
                        let qualified = join_path(&join_path(prefix, receiver_type), name);
                        push(out, NodeKind::Function, qualified, child, false);
                    }
                }
            }
            "type_declaration" => {
                if let Some(name) = type_declaration_name(child, source) {
                    push(out, NodeKind::Type, join_path(prefix, name), child, false);
                }
            }
            _ => {}
        }
    }
}

/// Walks the same top-level shapes as `walk`, but descends into function/method bodies
/// (which `walk` deliberately doesn't) to find `call_expression`s, recording each as a
/// `RefDecl` from the enclosing function. `current_fn` is `None` outside any function
/// body, matching every other adapter's `collect_refs`.
fn collect_refs(
    node: Node,
    source: &[u8],
    prefix: &str,
    current_fn: Option<&str>,
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_refs(body, source, prefix, Some(&qualified), out);
                    }
                }
            }
            "method_declaration" => {
                if let (Some(receiver), Some(name)) = (
                    child.child_by_field_name("receiver"),
                    field_text(child, "name", source),
                ) {
                    if let Some(receiver_type) = last_identifier_text(receiver, source) {
                        let qualified = join_path(&join_path(prefix, receiver_type), name);
                        if let Some(body) = child.child_by_field_name("body") {
                            collect_refs(body, source, prefix, Some(&qualified), out);
                        }
                    }
                }
            }
            "call_expression" => {
                if let (Some(from), Some(func)) =
                    (current_fn, child.child_by_field_name("function"))
                {
                    if let Some(name) = last_identifier_text(func, source) {
                        out.push(RefDecl {
                            from_qualified_path: from.to_string(),
                            to_name: name.to_string(),
                            kind: EdgeKind::Calls,
                        });
                    }
                }
                if let Some(args) = child.child_by_field_name("arguments") {
                    collect_refs(args, source, prefix, current_fn, out);
                }
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, out);
            }
        }
    }
}

/// Walks the whole file looking for `net/http` route registrations. Unlike `collect_refs`,
/// this doesn't need to track an enclosing function: a route's `symbol_name` is the
/// handler being registered, not whichever function happens to make the registration
/// call, so a plain recursive walk (no prefix/current-function bookkeeping) is enough.
fn collect_contracts(
    node: Node,
    source: &[u8],
    config: &DetectorConfig,
    out: &mut Vec<ContractRef>,
) {
    if node.kind() == "call_expression" && config.api_frameworks.iter().any(|f| f == "net/http") {
        if let Some((verb, path, handler)) = net_http_route_call(node, source) {
            out.push(ContractRef {
                contract_kind: ContractKind::ApiRoute,
                contract_id: format!("{verb} {path}"),
                symbol_name: handler,
                role: ContractRole::Produces,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_contracts(child, source, config, out);
    }
}

/// A `mux.HandleFunc("METHOD /path", handler)` registration, if `call` is one — see the
/// module doc for why only the method-prefixed (Go 1.22+) pattern form is recognized.
fn net_http_route_call(call: Node, source: &[u8]) -> Option<(String, String, String)> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "selector_expression" {
        return None;
    }
    let field = field_text(function, "field", source)?;
    if field != "HandleFunc" {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let pattern_arg = arguments.named_child(0)?;
    let pattern = string_literal_text(pattern_arg, source)?;
    let (verb, path) = pattern.split_once(' ')?;
    if !HTTP_VERBS.contains(&verb) {
        return None;
    }

    let handler_arg = arguments.named_child(1)?;
    let handler = last_identifier_text(handler_arg, source)?.to_string();

    Some((verb.to_string(), path.to_string(), handler))
}

/// Strips the surrounding quotes from a Go interpreted string literal's raw source text.
/// Doesn't handle escape sequences or raw (backtick-quoted) string literals — route
/// patterns in practice need neither.
fn string_literal_text(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "interpreted_string_literal" {
        return None;
    }
    let text = node.utf8_text(source).ok()?;
    Some(text.trim_matches('"').to_string())
}
