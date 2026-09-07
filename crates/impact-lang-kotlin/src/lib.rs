//! A Kotlin `LanguageAdapter` (Android — Java is explicitly out of scope). Mirrors every
//! other adapter's structure and scope boundaries (see `impact-lang-ts`'s module doc):
//! functions, classes, and methods (`extract_symbols`), calls (`extract_references`), no
//! contract detection (`extract_contract_refs` always returns empty — an honest empty
//! result, not a stub).
//!
//! One real difference from every other adapter, found via a real parse-tree dump before
//! writing this: this grammar doesn't expose a function's/class's body as a named field
//! at all — `child_by_field_name("body")` returns `None` for both. The body is a
//! positional child (`function_body` wrapping a `block`, or `class_body` directly), found
//! here by scanning for that child *kind* instead (`first_child_of_kind`), the same
//! technique `impact-lang-go` already needed for `type_declaration`'s name.
//!
//! Detects tests via the `@Test` annotation (JUnit, the standard on Android/Kotlin) —
//! unlike TypeScript's fragmented Jest/Vitest/Mocha situation, `@Test` is a single,
//! unambiguous, near-universal convention here, so (like Python's/Go's naming
//! conventions) it's cheap enough to wire up. The annotation shows up as an inline
//! `modifiers` child of the function declaration itself, not a preceding sibling the way
//! Rust's `#[test]` does.

use std::path::Path;

use impact_core::{
    ContractRef, EdgeKind, FileAst, FileScope, LanguageAdapter, NodeKind, RefDecl, RefTarget,
    SymbolDecl,
};
use tree_sitter::Node;

#[derive(Default)]
pub struct KotlinAdapter;

impl KotlinAdapter {
    fn language() -> tree_sitter::Language {
        tree_sitter_kotlin_ng::LANGUAGE.into()
    }
}

impl LanguageAdapter for KotlinAdapter {
    fn language_id(&self) -> &'static str {
        "kotlin"
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.kt"]
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
        let source = ast.source.as_bytes();
        let scope = build_scope(ast.tree.root_node(), source, &prefix);
        let mut out = Vec::new();
        collect_refs(
            ast.tree.root_node(),
            source,
            &prefix,
            None,
            &scope,
            &mut out,
        );
        out
    }

    fn extract_contract_refs(&self, _ast: &FileAst) -> Vec<ContractRef> {
        Vec::new()
    }
}

/// Derives a module path from a file path relative to the project root, e.g.
/// `src/payment/Service.kt` -> `payment::Service`. Same approximation and rationale as
/// every other adapter's `module_prefix`: doesn't follow Gradle source sets or Kotlin
/// package declarations (which, like Go's, are logically per-directory rather than
/// per-file) — good enough for structural blast-radius.
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let path = path.strip_prefix("src/").unwrap_or(&path);
    let path = path.strip_suffix(".kt").unwrap_or(path);
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

/// This grammar doesn't field-name a declaration's body — see the module doc. Finds the
/// first direct child of the given kind instead.
fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| c.kind() == kind);
    found
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

/// Whether `function_decl` carries a `@Test`-shaped annotation (matches `@Test`,
/// `@org.junit.jupiter.api.Test`, or any other qualified path ending in `Test` — the
/// rightmost segment is what `last_identifier_text` extracts, so JUnit4 and JUnit5's
/// differently-qualified annotations are both recognized uniformly).
fn has_test_annotation(function_decl: Node, source: &[u8]) -> bool {
    let Some(modifiers) = first_child_of_kind(function_decl, "modifiers") else {
        return false;
    };
    let mut cursor = modifiers.walk();
    let has_test = modifiers
        .children(&mut cursor)
        .any(|m| m.kind() == "annotation" && last_identifier_text(m, source) == Some("Test"));
    has_test
}

/// The rightmost identifier-like leaf in an expression: `foo` for `foo()`, `method` for
/// `t.method()`, `Test` for `@Test` or `@org.junit.jupiter.api.Test`. Same structural,
/// non-type-aware approach as every other adapter's `last_identifier_text`.
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

/// Walks top-level declarations and class bodies, extracting one `SymbolDecl` per
/// function, class, and method (Kotlin reuses `function_declaration` for both, so no
/// separate method-kind handling is needed — just recursion into a class's body). Doesn't
/// descend into function bodies — nested declarations are out of scope, matching every
/// other adapter's `walk`.
fn walk(node: Node, source: &[u8], prefix: &str, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(
                        out,
                        NodeKind::Function,
                        prefix,
                        name,
                        child,
                        has_test_annotation(child, source),
                    );
                }
            }
            "class_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if let Some(body) = first_child_of_kind(child, "class_body") {
                        let new_prefix = join_path(prefix, name);
                        walk(body, source, &new_prefix, out);
                    }
                }
            }
            "property_declaration" => {
                if let Some(name) = top_level_binding(child, source) {
                    push(out, NodeKind::Field, prefix, name, child, false);
                }
            }
            _ => {}
        }
    }
}

/// A top-level property whose initializer *calls* something runs that call when the
/// module loads, which makes the property a call site with a name — the only name there
/// is to attribute the call to, since no function encloses it. Without this,
/// `val CLIENT = buildClient()` disappeared from `buildClient`'s blast radius.
///
/// A property that calls nothing (`val LIMIT = 10`) isn't a call site and isn't indexed:
/// this is about not losing edges, not about cataloguing constants.
///
/// The name is the first identifier under the declaration's binding pattern, which a real
/// parse-tree dump confirms comes before the `=` and so before the callee's own
/// identifier — this must not pick up `compute` from `val STARTUP = compute()`.
fn top_level_binding<'a>(declaration: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let mut value_runs_a_call = false;
    let mut past_equals = false;
    let mut cursor = declaration.walk();
    for child in declaration.children(&mut cursor) {
        if child.kind() == "=" {
            past_equals = true;
            continue;
        }
        if past_equals && contains_call(child) {
            value_runs_a_call = true;
            break;
        }
    }
    if !value_runs_a_call {
        return None;
    }
    let mut cursor = declaration.walk();
    for child in declaration.children(&mut cursor) {
        if child.kind() == "=" {
            break;
        }
        if let Some(name) = first_identifier(child, source) {
            return Some(name);
        }
    }
    None
}

/// The first identifier-like leaf in a subtree.
fn first_identifier<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if matches!(node.kind(), "identifier" | "simple_identifier") {
        return node.utf8_text(source).ok();
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = first_identifier(child, source) {
            return Some(found);
        }
    }
    None
}

/// Whether this subtree performs a call — the test for "does this initializer do work?".
fn contains_call(node: Node) -> bool {
    if node.kind() == "call_expression" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_call(child) {
            return true;
        }
    }
    false
}

/// The package directory a file belongs to — its module path minus the filename. Kotlin
/// scopes by package, so a name with no import behind it is most likely declared in a
/// sibling file of the same directory rather than unresolvable.
fn package_module(prefix: &str) -> String {
    let mut segments: Vec<&str> = prefix.split("::").filter(|s| !s.is_empty()).collect();
    segments.pop();
    segments.join("::")
}

/// Reads this file's `import` headers and top-level declarations into a `FileScope`.
///
/// Imports are read from the header's source text rather than by node kind: the exact
/// shape of an import in this grammar varies with what's being imported (a class, a
/// top-level function, an alias, a star), while the text is always `import a.b.c` or
/// `import a.b.*`, which is all this needs.
fn build_scope(root: Node, source: &[u8], prefix: &str) -> FileScope {
    let package = package_module(prefix);
    let mut scope = FileScope::new(prefix, RefTarget::Module(package));
    collect_scope_entries(root, source, &mut scope);
    scope
}

fn collect_scope_entries(node: Node, source: &[u8], scope: &mut FileScope) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            kind if kind.contains("import") => {
                if let Ok(text) = child.utf8_text(source) {
                    add_import_from_text(text, scope);
                }
            }
            "function_declaration" | "class_declaration" | "object_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    scope.declare_local(name);
                }
            }
            // The header list and the source file's top level both wrap what we want.
            _ => collect_scope_entries(child, source, scope),
        }
    }
}

/// Parses one `import a.b.name` / `import a.b.*` / `import a.b.name as alias` header.
fn add_import_from_text(text: &str, scope: &mut FileScope) {
    let Some(path) = text.trim().strip_prefix("import") else {
        return;
    };
    let path = path.trim();
    let (path, alias) = match path.split_once(" as ") {
        Some((path, alias)) => (path.trim(), Some(alias.trim())),
        None => (path, None),
    };
    let mut segments: Vec<&str> = path.split('.').map(str::trim).collect();
    let Some(last) = segments.pop() else {
        return;
    };
    let module = segments.join("::");
    if last == "*" {
        scope.add_wildcard(module);
        return;
    }
    scope.add_import(alias.unwrap_or(last), module);
}

/// Where a call's callee points: a bare name (this package unless an import says
/// otherwise), `this.method()`, or a call through an imported name. A call on any other
/// receiver has no declared type here to look up.
fn call_target(callee: Node, source: &[u8], scope: &FileScope) -> RefTarget {
    let Ok(text) = callee.utf8_text(source) else {
        return RefTarget::Opaque;
    };
    match text.split_once('.') {
        None => scope.bare(text.trim()),
        Some(("this", _)) => scope.own(),
        Some((receiver, _)) => scope.qualified(receiver.trim()),
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
    scope: &FileScope,
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = first_child_of_kind(child, "function_body") {
                        collect_refs(body, source, prefix, Some(&qualified), scope, out);
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
                            to_target: call_target(func, source, scope),
                        });
                    }
                }
                collect_refs(child, source, prefix, current_fn, scope, out);
            }
            "class_declaration" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    first_child_of_kind(child, "class_body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_refs(body, source, &new_prefix, current_fn, scope, out);
                }
            }
            // At the top level (`current_fn` is `None`) a property's initializer is the
            // only named thing its calls can belong to.
            "property_declaration" if current_fn.is_none() => {
                match top_level_binding(child, source) {
                    Some(name) => {
                        let qualified = join_path(prefix, name);
                        collect_refs(child, source, prefix, Some(&qualified), scope, out);
                    }
                    None => collect_refs(child, source, prefix, current_fn, scope, out),
                }
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, scope, out);
            }
        }
    }
}
