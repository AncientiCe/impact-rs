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

use std::path::Path;

use impact_core::{ContractRef, EdgeKind, FileAst, LanguageAdapter, NodeKind, RefDecl, SymbolDecl};
use tree_sitter::Node;

#[derive(Default)]
pub struct PythonAdapter;

impl PythonAdapter {
    fn language() -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
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

    fn extract_contract_refs(&self, _ast: &FileAst) -> Vec<ContractRef> {
        Vec::new()
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
