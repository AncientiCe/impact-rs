//! A Swift `LanguageAdapter`. Mirrors every other adapter's structure and scope
//! boundaries (see `impact-lang-ts`'s module doc): functions, classes, and methods
//! (`extract_symbols`), calls (`extract_references`), no contract detection
//! (`extract_contract_refs` always returns empty — an honest empty result, not a stub).
//!
//! Sequenced last of the five languages added in this batch, deliberately: `tree-sitter-
//! swift` is community-maintained, not an official tree-sitter-org grammar, so it's the
//! least battle-tested of the five — real parse-tree dumps were checked before writing
//! any extraction code, same as every other adapter, and turned up one real API quirk:
//! `call_expression` has no `function`/`arguments` fields at all (unlike most of this
//! grammar family, where `function` at least exists even when other fields don't) — the
//! callee is purely positional (`child(0)`), so this adapter recurses into the whole
//! `call_expression` node generically rather than field-extracting its arguments,
//! matching `impact-lang-kotlin`'s same defensive choice for the same reason.
//!
//! Detects tests via XCTest's real convention: a `test`-prefixed method inside a class
//! that inherits `XCTestCase` (found via the `inheritance_specifier` child every
//! subclass/conformance produces) — not just any `test`-prefixed method anywhere, which
//! would be too broad given how ordinary a word "test" is as a method-name prefix outside
//! that specific context.

use std::path::Path;

use impact_core::{ContractRef, EdgeKind, FileAst, LanguageAdapter, NodeKind, RefDecl, SymbolDecl};
use tree_sitter::Node;

#[derive(Default)]
pub struct SwiftAdapter;

impl SwiftAdapter {
    fn language() -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }
}

impl LanguageAdapter for SwiftAdapter {
    fn language_id(&self) -> &'static str {
        "swift"
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.swift"]
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
            false,
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
/// `Sources/Payment/Service.swift` -> `Payment::Service`. Same approximation and
/// rationale as every other adapter's `module_prefix`: doesn't follow Swift Package
/// Manager target boundaries or module maps — good enough for structural blast-radius.
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let path = path.strip_suffix(".swift").unwrap_or(&path);
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
    prefix: &str,
    name: &str,
    node: Node,
    is_test: bool,
) {
    out.push(SymbolDecl {
        kind,
        qualified_path: join_path(prefix, name),
        line: node.start_position().row + 1,
        is_test,
    });
}

/// Whether `class_decl` conforms to/inherits `XCTestCase`, via its `inheritance_specifier`
/// children (Swift doesn't distinguish superclass from protocol conformance
/// syntactically, so this matches either).
fn inherits_xctest_case(class_decl: Node, source: &[u8]) -> bool {
    let mut cursor = class_decl.walk();
    let inherits = class_decl.children(&mut cursor).any(|c| {
        c.kind() == "inheritance_specifier" && last_identifier_text(c, source) == Some("XCTestCase")
    });
    inherits
}

/// The rightmost identifier-like leaf in an expression: `foo` for `foo()`, `method` for
/// `t.method()`, `XCTestCase` for an inheritance specifier. Same structural,
/// non-type-aware approach as every other adapter's `last_identifier_text`.
fn last_identifier_text<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if matches!(node.kind(), "simple_identifier" | "type_identifier") {
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

/// Walks top-level declarations and class bodies, extracting one `SymbolDecl` per
/// function, class, and method (Swift reuses `function_declaration` for both, so no
/// separate method-kind handling is needed — just recursion into a class's `body`).
/// Doesn't descend into function bodies — nested declarations are out of scope, matching
/// every other adapter's `walk`. `in_xctest_case` tracks whether the current class
/// inherits `XCTestCase`, for `test`-prefixed method detection.
fn walk(node: Node, source: &[u8], prefix: &str, in_xctest_case: bool, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    let is_test = in_xctest_case && name.starts_with("test");
                    push(out, NodeKind::Function, prefix, name, child, is_test);
                }
            }
            "class_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if let Some(body) = child.child_by_field_name("body") {
                        let new_prefix = join_path(prefix, name);
                        let is_xctest = inherits_xctest_case(child, source);
                        walk(body, source, &new_prefix, is_xctest, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Walks the same declaration shapes as `walk`, but descends into function/method bodies
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
            "call_expression" => {
                if let (Some(from), Some(func)) = (current_fn, child.child(0)) {
                    if let Some(name) = last_identifier_text(func, source) {
                        out.push(RefDecl {
                            from_qualified_path: from.to_string(),
                            to_name: name.to_string(),
                            kind: EdgeKind::Calls,
                        });
                    }
                }
                collect_refs(child, source, prefix, current_fn, out);
            }
            "class_declaration" => {
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
