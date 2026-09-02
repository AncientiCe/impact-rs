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
//! (`extract_references`). Detects tests by file-naming convention only (`is_test_file`)
//! — every JS/TS test framework's own *call*-based marker (`test()`/`it()`, `describe`
//! blocks) varies enough between Jest/Vitest/Mocha that guessing at one would be worse
//! than not detecting it, but `*.test.*`/`*.spec.*`/`__tests__/` is a naming convention
//! all three frameworks (and their default test-runner configs) actually share.
//!
//! Also detects one API contract shape (gated on `impact.toml`'s `api_frameworks`
//! containing `"express"` and/or `"fastify"`, both on by default): an
//! `app.get(path, handler)`-style route registration call — Express's own method-chaining
//! API and Fastify's shortcut methods share the exact same call shape (`app`/`fastify`/
//! `router`, any receiver name; `.get`/`.post`/`.put`/`.delete`/`.patch`; a string path
//! then a handler reference), confirmed via a real parse-tree dump before writing this.
//! Only a plain named-function or `object.method` handler reference is recognized — an
//! inline arrow/function-expression handler has no name to report, so no route is
//! emitted for it rather than guessing at one of its inner identifiers.

use std::path::Path;

use impact_core::{
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, FileAst, LanguageAdapter,
    NodeKind, RefDecl, SymbolDecl,
};
use tree_sitter::Node;

const HTTP_VERBS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

pub struct TsAdapter {
    config: DetectorConfig,
}

impl TsAdapter {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

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

impl Default for TsAdapter {
    fn default() -> Self {
        Self::new(DetectorConfig::default())
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
        let is_test_file = is_test_file(&ast.path);
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

/// Whether every function/method declared in this file should be marked a test, by the
/// file-naming convention Jest, Vitest, and Mocha all share (unlike a call-based
/// convention like `test()`/`it()`, which varies enough between those frameworks that
/// this adapter otherwise avoids test detection entirely — see the module doc): the
/// filename itself contains a `.test.` or `.spec.` segment (`foo.test.ts`,
/// `foo.spec.tsx`), or the file lives under a `__tests__/` directory. A `test()` call
/// inside an otherwise-ordinarily-named file is deliberately not detected — that would
/// need call-site analysis this adapter doesn't do, not a path check.
fn is_test_file(rel_path: &str) -> bool {
    let path = rel_path.replace('\\', "/");
    if path.split('/').any(|segment| segment == "__tests__") {
        return true;
    }
    let Some(filename) = path.rsplit('/').next() else {
        return false;
    };
    filename.contains(".test.") || filename.contains(".spec.")
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

/// Walks top-level declarations (transparently unwrapping `export`/`export default`) and
/// class bodies, extracting one `SymbolDecl` per function, class, and method. Doesn't
/// descend into function bodies — nested declarations are out of scope, matching
/// `impact-lang-rust`'s `walk`. `is_test_file` marks every function/method `is_test`
/// (never a class itself) — see `is_test_file`'s own doc for the convention.
fn walk(node: Node, source: &[u8], prefix: &str, is_test_file: bool, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Function, prefix, name, child, is_test_file);
                }
            }
            "class_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if let Some(body) = child.child_by_field_name("body") {
                        let new_prefix = join_path(prefix, name);
                        walk(body, source, &new_prefix, is_test_file, out);
                    }
                }
            }
            "method_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Function, prefix, name, child, is_test_file);
                }
            }
            "export_statement" => {
                // `export function foo() {}` / `export class X {}` wrap the real
                // declaration one level down — unwrap transparently, same prefix.
                walk(child, source, prefix, is_test_file, out);
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

/// Strips the surrounding quotes from a `string` node's raw source text. Doesn't handle
/// template literals or escape sequences — a dynamic or composed path can't be resolved
/// structurally anyway, so this only ever needs to read a plain literal.
fn ts_string_text(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let text = node.utf8_text(source).ok()?;
    Some(
        text.trim_matches(|c| c == '"' || c == '\'' || c == '`')
            .to_string(),
    )
}

/// An `app.get(path, handler)`-style Express/Fastify route registration, if `call` is
/// one — see the module doc for why both frameworks share this detector. `handler` is
/// only recognized as a plain identifier or `object.method` reference; an inline
/// function/arrow-function handler yields `None` rather than guessing at a name.
fn express_route_call(call: Node, source: &[u8]) -> Option<(String, String, String)> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "member_expression" {
        return None;
    }
    let verb = field_text(function, "property", source)?.to_uppercase();
    if !HTTP_VERBS.contains(&verb.as_str()) {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let path_arg = arguments.named_child(0)?;
    let path = ts_string_text(path_arg, source)?;

    let handler_arg = arguments.named_child(1)?;
    if !matches!(handler_arg.kind(), "identifier" | "member_expression") {
        return None;
    }
    let handler = last_identifier_text(handler_arg, source)?.to_string();

    Some((verb, path, handler))
}

/// Walks the whole file looking for Express/Fastify route registrations. Unlike
/// `collect_refs`, this doesn't need to track an enclosing function or prefix: a route's
/// `symbol_name` is the handler being registered, not whichever function happens to make
/// the registration call, so a plain recursive walk is enough — matching
/// `impact-lang-go`'s `net/http` detector, which has the same shape of independence.
fn collect_contracts(
    node: Node,
    source: &[u8],
    config: &DetectorConfig,
    out: &mut Vec<ContractRef>,
) {
    if node.kind() == "call_expression"
        && config
            .api_frameworks
            .iter()
            .any(|f| f == "express" || f == "fastify")
    {
        if let Some((verb, path, handler)) = express_route_call(node, source) {
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
