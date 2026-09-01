//! A TypeScript/JavaScript `LanguageAdapter` — the second language adapter this project
//! ships, written specifically to prove `impact-core`'s adapter boundary actually holds:
//! nothing here required a single change to `impact-core`, `impact-cli`, the graph model,
//! the linker, the blast-radius engine, or the MCP surface. Register a `TsAdapter`
//! alongside `RustAdapter` in the same `Indexer` and both languages get indexed, queried,
//! and `--change`-resolved through the exact same machinery.
//!
//! Also covers React and React Native: both are TypeScript/JavaScript with JSX, not a
//! separate language, so `.tsx`/`.jsx`/`.js`/`.mjs` are handled by widening this same
//! adapter rather than writing a new one. Confirmed empirically before adding them:
//! `tree-sitter-typescript`'s TSX grammar parses plain JSX-containing JavaScript (no
//! TS-specific syntax at all) with zero parse errors, producing the exact same node kinds
//! (`function_declaration`, `call_expression` reachable inside a `jsx_element` via the
//! existing generic recursion below) this adapter already handles — so no new extraction
//! logic was needed, only `parse_file` choosing the right grammar per file. `.ts` files
//! still get the plain TypeScript grammar rather than TSX for all files uniformly,
//! because the two genuinely disagree on `<Foo>bar`: the TypeScript grammar accepts it as
//! a legacy type-assertion cast, while the TSX grammar must treat a leading `<` as the
//! start of a JSX element to support JSX at all — real syntax used in real `.ts` files
//! that predates `as Foo`, so `.ts` keeps the grammar that doesn't misparse it.
//!
//! Deliberately scoped down relative to `impact-lang-rust`: functions, classes, and
//! methods (`extract_symbols`), and calls including simple method calls
//! (`extract_references`). No contract detection (`extract_contract_refs` always returns
//! empty — this adapter doesn't claim to recognize any TS API/event/database framework,
//! which is an honest empty result, not a stub pretending to work) and no test-attribute
//! detection (JS/TS test conventions — Jest, Vitest, Mocha — vary enough that guessing
//! wrong would be worse than always reporting `is_test: false`).

use std::path::Path;

use impact_core::{ContractRef, EdgeKind, FileAst, LanguageAdapter, NodeKind, RefDecl, SymbolDecl};
use tree_sitter::Node;

#[derive(Default)]
pub struct TsAdapter;

impl TsAdapter {
    /// `.ts` gets the plain TypeScript grammar (see the module doc for why); every other
    /// extension this adapter claims (`.tsx`/`.jsx`/`.js`/`.mjs`) gets the TSX grammar,
    /// which is a strict enough superset to parse plain JS/JSX cleanly too.
    fn language_for(path: &Path) -> tree_sitter::Language {
        if path.extension().and_then(|e| e.to_str()) == Some("ts") {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        } else {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        }
    }
}

impl LanguageAdapter for TsAdapter {
    fn language_id(&self) -> &'static str {
        "typescript"
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.ts", "**/*.tsx", "**/*.jsx", "**/*.js", "**/*.mjs"]
    }

    fn parse_file(&self, path: &Path, source: &str) -> anyhow::Result<FileAst> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&Self::language_for(path))?;
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
/// `src/payment/service.ts` -> `payment::service`. Same approximation and same rationale
/// as `impact-lang-rust`'s `module_prefix`: good enough for structural blast-radius, not
/// a claim of real module-resolution correctness (doesn't follow `tsconfig.json` path
/// aliases or barrel-file re-exports).
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let path = path.strip_prefix("src/").unwrap_or(&path);
    let path = [".tsx", ".ts", ".jsx", ".mjs", ".js"]
        .iter()
        .find_map(|ext| path.strip_suffix(ext))
        .unwrap_or(path);
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

fn push(out: &mut Vec<SymbolDecl>, kind: NodeKind, prefix: &str, name: &str, node: Node) {
    out.push(SymbolDecl {
        kind,
        qualified_path: join_path(prefix, name),
        line: node.start_position().row + 1,
        is_test: false,
    });
}

/// Walks top-level declarations (transparently unwrapping `export`/`export default`) and
/// class bodies, extracting one `SymbolDecl` per function, class, and method. Doesn't
/// descend into function bodies — nested declarations are out of scope, matching
/// `impact-lang-rust`'s `walk`.
fn walk(node: Node, source: &[u8], prefix: &str, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Function, prefix, name, child);
                }
            }
            "class_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child);
                    if let Some(body) = child.child_by_field_name("body") {
                        let new_prefix = join_path(prefix, name);
                        walk(body, source, &new_prefix, out);
                    }
                }
            }
            "method_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Function, prefix, name, child);
                }
            }
            "export_statement" => {
                // `export function foo() {}` / `export class X {}` wrap the real
                // declaration one level down — unwrap transparently, same prefix.
                walk(child, source, prefix, out);
            }
            _ => {}
        }
    }
}

/// The rightmost identifier-like leaf in a callee expression: `foo` for `foo()`, `method`
/// for `t.method()` (a `member_expression`). Same structural, non-type-aware approach as
/// `impact-lang-rust`'s `last_identifier_text`, for the same reason: it's the name the
/// linker needs to match, not a claim of knowing what `t` resolves to.
fn last_identifier_text<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if matches!(
        node.kind(),
        "identifier" | "property_identifier" | "type_identifier"
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

/// Walks the same declaration shapes as `walk`, but descends into function/method bodies
/// (which `walk` deliberately doesn't) to find `call_expression`s, recording each as a
/// `RefDecl` from the enclosing function. `current_fn` is `None` outside any function
/// body, matching `impact-lang-rust`'s `collect_refs`.
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
            "function_declaration" | "method_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_refs(body, source, prefix, Some(&qualified), out);
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
