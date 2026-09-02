//! A Python `LanguageAdapter`. Mirrors `impact-lang-ts`'s structure and scope
//! boundaries exactly (see its module doc): functions, classes, and methods
//! (`extract_symbols`), calls including method calls (`extract_references`), no
//! contract detection (`extract_contract_refs` always returns empty — an honest empty
//! result, not a stub). Confirmed via a real parse-tree dump before writing this: Python
//! reuses one node kind (`function_definition`) for both free functions and methods (no
//! separate "method" kind the way Go has `method_declaration`), so the symbol walker
//! needs no impl-block-style special case — just recursion into a class's `body`.
//!
//! Unlike TypeScript, this adapter *does* detect tests: pytest's actual discovery rule is
//! "any function/method whose name starts with `test`" (no framework-specific attribute
//! or decorator required, unlike Jest/Vitest/Mocha's need-a-call-to-`test()`/`it()`
//! ambiguity that made TypeScript skip this) — cheap, unambiguous, and matches both
//! plain pytest functions and `unittest.TestCase` methods (which follow the same
//! `test`-prefix convention).
//!
//! Also detects one API contract shape (gated on `impact.toml`'s `api_frameworks`
//! containing `"fastapi"` and/or `"flask"`, both on by default): a route decorator
//! directly above a `def`, e.g. `@app.get("/payments")` (FastAPI, and Flask 2.0+'s
//! verb-method aliases) or `@app.route("/payments", methods=["POST"])` (Flask's
//! original form — verb read from the first string in `methods=[...]`, defaulting to
//! `GET` when the keyword is absent, matching Flask's own default). Confirmed via a real
//! parse-tree dump before writing this: a decorator's argument list holds positional
//! `string` nodes and `keyword_argument` nodes side by side, same shape either form
//! needs to read. The decorated function *is* the handler — unlike axum/net/http's
//! `.route(path, verb(handler))` call, there's no separate handler argument to extract,
//! so `symbol_name` is just the decorated function's own qualified path.

use std::path::Path;

use impact_core::{
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, FileAst, LanguageAdapter,
    NodeKind, RefDecl, SymbolDecl,
};
use tree_sitter::Node;

const HTTP_VERBS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

pub struct PythonAdapter {
    config: DetectorConfig,
}

impl PythonAdapter {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

    fn language() -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }
}

impl Default for PythonAdapter {
    fn default() -> Self {
        Self::new(DetectorConfig::default())
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.py"]
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
        let mut out = Vec::new();
        walk(
            ast.tree.root_node(),
            ast.source.as_bytes(),
            &prefix,
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
        let prefix = module_prefix(&ast.path);
        let mut out = Vec::new();
        collect_contracts(
            ast.tree.root_node(),
            ast.source.as_bytes(),
            &prefix,
            &self.config,
            &mut out,
        );
        out
    }
}

/// Derives a module path from a file path relative to the project root, e.g.
/// `src/payment/service.py` -> `payment::service`. `__init__.py` is special-cased to
/// represent its own directory (the package), not an `__init__` submodule — the same
/// treatment `impact-lang-rust` gives `mod.rs`. Same approximation and rationale as every
/// other adapter's `module_prefix`: doesn't follow `sys.path` manipulation, namespace
/// packages, or relative-import resolution — good enough for structural blast-radius.
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let path = path.strip_prefix("src/").unwrap_or(&path);
    let path = path.strip_suffix(".py").unwrap_or(path);
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.last() == Some(&"__init__") {
        segments.pop();
    }
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
    prefix: &str,
    name: &str,
    node: Node,
    is_test: bool,
) {
    out.push(SymbolDecl {
        kind,
        qualified_path: join_path(prefix, name),
        line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        is_test,
    });
}

/// pytest's actual discovery rule, applied identically to free functions and methods:
/// any name starting with `test` — no decorator or base-class check required. This also
/// happens to match `unittest.TestCase` methods, which follow the same naming
/// convention.
fn is_pytest_name(name: &str) -> bool {
    name.starts_with("test")
}

/// Walks top-level declarations and class bodies, extracting one `SymbolDecl` per
/// function, class, and method. Doesn't descend into function bodies — nested
/// declarations are out of scope, matching every other adapter's `walk`.
fn walk(node: Node, source: &[u8], prefix: &str, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(
                        out,
                        NodeKind::Function,
                        prefix,
                        name,
                        child,
                        is_pytest_name(name),
                    );
                }
            }
            "class_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if let Some(body) = child.child_by_field_name("body") {
                        let new_prefix = join_path(prefix, name);
                        walk(body, source, &new_prefix, out);
                    }
                }
            }
            // `@decorator\ndef foo(): ...` wraps the real definition one level down —
            // unwrap transparently, same as TypeScript's `export_statement` handling.
            "decorated_definition" => {
                walk(child, source, prefix, out);
            }
            _ => {}
        }
    }
}

/// The rightmost identifier-like leaf in a callee expression: `foo` for `foo()`, `method`
/// for `t.method()` (an `attribute` node). Same structural, non-type-aware approach as
/// every other adapter's `last_identifier_text`.
fn last_identifier_text<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if node.kind() == "identifier" {
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

/// Walks the same declaration shapes as `walk`, but descends into function/method bodies
/// (which `walk` deliberately doesn't) to find `call`s, recording each as a `RefDecl`
/// from the enclosing function. `current_fn` is `None` outside any function body,
/// matching every other adapter's `collect_refs`.
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
            "function_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_refs(body, source, prefix, Some(&qualified), out);
                    }
                }
            }
            "call" => {
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
            "class_definition" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_refs(body, source, &new_prefix, current_fn, out);
                }
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, out);
            }
        }
    }
}

fn find_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// Strips the surrounding quotes from a Python `string` node's raw source text. Doesn't
/// handle f-strings, concatenation, or escape sequences — a dynamic or composed path
/// can't be resolved structurally anyway, so this only ever needs to read a plain literal.
fn python_string_text(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let text = node.utf8_text(source).ok()?;
    Some(text.trim_matches(|c| c == '"' || c == '\'').to_string())
}

/// The first positional (non-keyword) argument, if it's a plain string literal — the
/// route path in every recognized decorator shape. Positional arguments always precede
/// keyword ones in Python call syntax, so the first named child that isn't a
/// `keyword_argument` is either the path or, if it's not a `string`, an expression this
/// adapter can't resolve structurally (never guessed at).
fn first_positional_string(arguments: Node, source: &[u8]) -> Option<String> {
    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            continue;
        }
        return python_string_text(child, source);
    }
    None
}

/// Flask's `methods=[...]` keyword argument on `@app.route(...)`, if present: the first
/// string in the list. Flask accepts multiple methods per route; this adapter only
/// surfaces the first, matching every other adapter's "one clear case, not exhaustive
/// coverage" scope for a feature that's rare in practice (most routes list one method).
fn flask_methods_keyword(arguments: Node, source: &[u8]) -> Option<String> {
    let mut cursor = arguments.walk();
    let keyword = arguments.named_children(&mut cursor).find(|c| {
        c.kind() == "keyword_argument" && field_text(*c, "name", source) == Some("methods")
    })?;
    let value = keyword.child_by_field_name("value")?;
    if value.kind() != "list" {
        return None;
    }
    let first_string = find_child_of_kind(value, "string")?;
    python_string_text(first_string, source)
}

/// A FastAPI/Flask route decorator, if `decorator` is one: `@app.get(path)`-style verb
/// aliases (both frameworks), or Flask's `@app.route(path, methods=[...])`. Returns the
/// HTTP verb (uppercased) and path.
fn python_route_decorator(
    decorator: Node,
    source: &[u8],
    config: &DetectorConfig,
) -> Option<(String, String)> {
    let call = find_child_of_kind(decorator, "call")?;
    let function = call.child_by_field_name("function")?;
    if function.kind() != "attribute" {
        return None;
    }
    let method_name = field_text(function, "attribute", source)?;
    let arguments = call.child_by_field_name("arguments")?;
    let path = first_positional_string(arguments, source)?;

    if method_name == "route" {
        if !config.api_frameworks.iter().any(|f| f == "flask") {
            return None;
        }
        let verb = flask_methods_keyword(arguments, source).unwrap_or_else(|| "GET".to_string());
        return Some((verb, path));
    }

    let verb = method_name.to_uppercase();
    if !HTTP_VERBS.contains(&verb.as_str()) {
        return None;
    }
    if !config
        .api_frameworks
        .iter()
        .any(|f| f == "fastapi" || f == "flask")
    {
        return None;
    }
    Some((verb, path))
}

/// Walks the same declaration shapes as `walk`, looking for a route decorator directly
/// above a `def` (see the module doc). Doesn't descend into function bodies — a route
/// registration always decorates a top-level or class-body function, matching every
/// other adapter's contract-detection scope.
fn collect_contracts(
    node: Node,
    source: &[u8],
    prefix: &str,
    config: &DetectorConfig,
    out: &mut Vec<ContractRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "decorated_definition" => {
                if let Some(func_def) = find_child_of_kind(child, "function_definition") {
                    if let Some(name) = field_text(func_def, "name", source) {
                        let qualified = join_path(prefix, name);
                        let mut dec_cursor = child.walk();
                        for decorator in child
                            .children(&mut dec_cursor)
                            .filter(|c| c.kind() == "decorator")
                        {
                            if let Some((verb, path)) =
                                python_route_decorator(decorator, source, config)
                            {
                                out.push(ContractRef {
                                    contract_kind: ContractKind::ApiRoute,
                                    contract_id: format!("{verb} {path}"),
                                    symbol_name: qualified.clone(),
                                    role: ContractRole::Produces,
                                });
                            }
                        }
                    }
                }
            }
            "class_definition" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_contracts(body, source, &new_prefix, config, out);
                }
            }
            _ => {}
        }
    }
}
