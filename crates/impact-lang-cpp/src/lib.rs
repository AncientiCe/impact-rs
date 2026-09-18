//! A C++ `LanguageAdapter`. Mirrors every other adapter's structure and scope boundaries
//! (see `impact-lang-swift`'s module doc): functions, classes/structs, and methods
//! (`extract_symbols`), calls (`extract_references`), no test detection and no contract
//! detection (`extract_contract_refs` always returns empty — an honest empty result, not
//! a stub). Real parse-tree dumps (`tree_sitter_cpp`, via `.to_sexp()`) were checked
//! before writing any extraction code, same as every other adapter.
//!
//! Two things make C++ structurally different from every adapter added so far:
//!
//! - **Out-of-line definitions.** The dominant real-world pattern is a class declared in
//!   a header (`class Foo { void bar(); };`, a body-less prototype — never indexed, since
//!   no adapter indexes a signature it can't attribute a location worth reporting to) and
//!   defined in a `.cpp` file as `void Foo::bar() { ... }`. That definition's declarator
//!   is a `qualified_identifier` (`scope: Foo, name: bar`), not the bare `identifier` a
//!   free function gets — `declarator_name` below reads its whole source text (`"Foo::bar"`,
//!   or `"a::b::Foo::bar"` for a nested namespace) as the qualified-path suffix directly,
//!   rather than trying to re-derive segments from the parse tree.
//! - **No file-scoped import system.** `#include` pulls in header *text*, not a
//!   `module -> names` binding an adapter could read the way `use`/`import` can
//!   elsewhere — a name declared `extern` (the C++ default) is visible, and callable by a
//!   bare name, from any translation unit that merely declares it. So unlike Rust or
//!   TypeScript (where an unresolved bare name is `Opaque` — something this file never
//!   imported isn't this file's caller) a C++ file's unresolved bare name gets `Unscoped`,
//!   the same choice `impact-lang-swift` makes for the same underlying reason: structural
//!   resolution project-wide is the best evidence available, not a guess standing in for
//!   real scope information.
//!
//! GoogleTest's `TEST`/`TEST_F` macros happen to parse as an ordinary-looking
//! `function_definition` (tree-sitter has no preprocessor and doesn't expand them), but
//! relying on that shape would be guessing at one specific framework's macro expansion
//! rather than reading real C++ syntax — left undetected, same call as `is_generated`/
//! `is_default_export` staying `false` for every adapter without a structural way to know.

use std::path::Path;

use impact_core::{
    ContractRef, EdgeKind, FileAst, FileScope, LanguageAdapter, NodeKind, RefDecl, RefTarget,
    SymbolDecl,
};
use tree_sitter::Node;

/// Source-file extensions this adapter claims, paired 1:1 with `file_globs`'s patterns —
/// kept as a single list so `module_prefix` strips exactly the suffixes the indexer
/// routed here.
const CPP_EXTENSIONS: &[&str] = &[".cpp", ".cc", ".cxx", ".c++", ".hpp", ".hh", ".hxx", ".h"];

#[derive(Default)]
pub struct CppAdapter;

impl CppAdapter {
    fn language() -> tree_sitter::Language {
        tree_sitter_cpp::LANGUAGE.into()
    }
}

impl LanguageAdapter for CppAdapter {
    fn language_id(&self) -> &'static str {
        "cpp"
    }

    fn file_globs(&self) -> &[&str] {
        &[
            "**/*.cpp", "**/*.cc", "**/*.cxx", "**/*.c++", "**/*.hpp", "**/*.hh", "**/*.hxx",
            "**/*.h",
        ]
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
/// `src/payment/service.cpp` -> `src::payment::service`. Same approximation and
/// rationale as every other adapter's `module_prefix`: doesn't follow the project's real
/// namespace layout — good enough for structural blast-radius, and namespaces
/// encountered while walking still nest correctly underneath it (see `walk`).
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let stem = CPP_EXTENSIONS
        .iter()
        .find_map(|ext| path.strip_suffix(ext))
        .unwrap_or(path.as_str());
    let segments: Vec<&str> = stem.split('/').filter(|s| !s.is_empty()).collect();
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

/// Finds the `function_declarator` inside a (possibly pointer-/reference-returning)
/// declarator. `pointer_declarator` exposes its inner declarator via a `declarator`
/// field; `reference_declarator` doesn't label it as a field at all (a real parse-tree
/// quirk confirmed via `.to_sexp()`), so that case falls back to the first named child.
fn find_function_declarator(node: Node) -> Option<Node> {
    if node.kind() == "function_declarator" {
        return Some(node);
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return find_function_declarator(inner);
    }
    if node.kind() == "reference_declarator" {
        return find_function_declarator(node.named_child(0)?);
    }
    None
}

/// The declared name of a `function_definition`, and whether it's already a full
/// qualified-path suffix (`Foo::bar`, from an out-of-line method definition) rather than
/// a single segment (a free function, an inline method, a constructor/destructor, or an
/// operator overload).
enum DeclaredName<'a> {
    Segment(&'a str),
    QualifiedSuffix(&'a str),
}

fn function_declared_name<'a>(def: Node, source: &'a [u8]) -> Option<DeclaredName<'a>> {
    let outer = def.child_by_field_name("declarator")?;
    let declarator = find_function_declarator(outer)?;
    let name_node = declarator.child_by_field_name("declarator")?;
    match name_node.kind() {
        "qualified_identifier" => Some(DeclaredName::QualifiedSuffix(
            name_node.utf8_text(source).ok()?,
        )),
        "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
            Some(DeclaredName::Segment(name_node.utf8_text(source).ok()?))
        }
        _ => None,
    }
}

fn push(out: &mut Vec<SymbolDecl>, kind: NodeKind, qualified_path: String, node: Node) {
    out.push(SymbolDecl {
        kind,
        qualified_path,
        line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        is_test: false,
        is_generated: false,
        is_default_export: false,
    });
}

/// Walks namespaces, classes/structs, and top-level declarations, extracting one
/// `SymbolDecl` per function definition (free function, inline method, or an out-of-line
/// `Type::method` definition) and one per class/struct. Doesn't descend into function
/// bodies — nested declarations are out of scope, matching every other adapter's `walk`.
/// `template_declaration` is transparent (its templated function/class is the only child
/// that matters here), so it recurses with the prefix unchanged.
fn walk(node: Node, source: &[u8], prefix: &str, out: &mut Vec<SymbolDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => match function_declared_name(child, source) {
                Some(DeclaredName::Segment(name)) => {
                    push(out, NodeKind::Function, join_path(prefix, name), child);
                }
                Some(DeclaredName::QualifiedSuffix(suffix)) => {
                    push(out, NodeKind::Function, join_path(prefix, suffix), child);
                }
                None => {}
            },
            "class_specifier" | "struct_specifier" => {
                let name = field_text(child, "name", source);
                let new_prefix = match name {
                    Some(name) => {
                        push(out, NodeKind::Type, join_path(prefix, name), child);
                        join_path(prefix, name)
                    }
                    None => prefix.to_string(),
                };
                if let Some(body) = child.child_by_field_name("body") {
                    walk(body, source, &new_prefix, out);
                }
            }
            "namespace_definition" => {
                let new_prefix = match field_text(child, "name", source) {
                    Some(name) => join_path(prefix, name),
                    None => prefix.to_string(),
                };
                if let Some(body) = child.child_by_field_name("body") {
                    walk(body, source, &new_prefix, out);
                }
            }
            "template_declaration" => walk(child, source, prefix, out),
            _ => {}
        }
    }
}

/// Reads this file's top-level function names, and every method every class/struct in
/// this file declares (regardless of which type), into a `FileScope` — the same
/// same-file-wide approximation `impact-lang-swift`'s `declare_scope_names` documents,
/// for the same reason: a bare call inside one method to a sibling method declared
/// elsewhere in the same class is C++'s implicit-`this` convention, and distinguishing
/// which class's method table a bare name belongs to would need real type resolution
/// this adapter doesn't do.
fn build_scope(root: Node, source: &[u8], prefix: &str) -> FileScope {
    let mut scope = FileScope::new(prefix, RefTarget::Unscoped);
    declare_scope_names(root, source, &mut scope);
    scope
}

fn declare_scope_names(node: Node, source: &[u8], scope: &mut FileScope) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(DeclaredName::Segment(name)) = function_declared_name(child, source) {
                    scope.declare_local(name);
                }
            }
            "class_specifier" | "struct_specifier" => {
                if let Some(name) = field_text(child, "name", source) {
                    scope.declare_local(name);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    declare_scope_names(body, source, scope);
                }
            }
            "namespace_definition" | "template_declaration" => {
                if let Some(body) = child.child_by_field_name("body") {
                    declare_scope_names(body, source, scope);
                } else {
                    declare_scope_names(child, source, scope);
                }
            }
            _ => {}
        }
    }
}

/// The leftmost identifier-like leaf in a subtree — the fallback for a callee shape none
/// of `call_target`'s explicit cases recognize (a template call like `foo<int>()`, a call
/// through a parenthesized expression, and similar). Same defensive, grammar-uncertainty
/// fallback every other adapter's `last_identifier_text`/generic callee recursion uses.
fn first_identifier_like<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if matches!(node.kind(), "identifier" | "field_identifier") {
        return node.utf8_text(source).ok();
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(text) = first_identifier_like(child, source) {
            return Some(text);
        }
    }
    None
}

/// The immediate qualifier a `qualified_identifier` callee's `scope` field names — the
/// nearest enclosing name before the final `::`, read off the parse tree's own fields
/// rather than by splitting text (a nested `a::b::Foo::bar` nests `scope` recursively, so
/// the "immediate" qualifier is `Foo`'s `name` field one level in, not `a`).
fn qualified_immediate_qualifier<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let scope = node.child_by_field_name("scope")?;
    if scope.kind() == "qualified_identifier" {
        field_text(scope, "name", source)
    } else {
        scope.utf8_text(source).ok()
    }
}

/// Where a call's callee points: a bare name falls through to file-wide resolution
/// (`Unscoped`, per the module doc); `this->method()`/`this.method()` is this file's own
/// module; `object.method()`/`object->method()` on any other receiver, and a qualifier
/// `call_target` doesn't recognize, is `Opaque` — this adapter doesn't do type inference,
/// so a receiver that isn't literally `this` names *something* this file's text doesn't
/// establish is in scope here.
fn call_target(callee: Node, source: &[u8], scope: &FileScope) -> (String, RefTarget) {
    match callee.kind() {
        "identifier" => {
            let name = callee.utf8_text(source).unwrap_or_default();
            (name.to_string(), scope.bare(name))
        }
        "field_expression" => {
            let name = field_text(callee, "field", source).unwrap_or_default();
            let receiver = field_text(callee, "argument", source).unwrap_or_default();
            let target = if receiver.trim() == "this" {
                scope.own()
            } else {
                scope.qualified(receiver.trim())
            };
            (name.to_string(), target)
        }
        "qualified_identifier" => {
            let name = field_text(callee, "name", source).unwrap_or_default();
            let target = match qualified_immediate_qualifier(callee, source) {
                Some(qualifier) => scope.qualified(qualifier),
                None => RefTarget::Opaque,
            };
            (name.to_string(), target)
        }
        _ => match first_identifier_like(callee, source) {
            Some(name) => (name.to_string(), scope.bare(name)),
            None => (String::new(), RefTarget::Opaque),
        },
    }
}

/// Walks the same declaration shapes as `walk`, but descends into function/method bodies
/// (which `walk` deliberately doesn't) to find `call_expression`s, recording each as a
/// `RefDecl` from the enclosing function. `current_fn` is `None` outside any function
/// body, matching every other adapter's `collect_refs`. Everything not explicitly
/// matched (namespaces, templates, statements, expressions) recurses generically with
/// `prefix`/`current_fn` unchanged, which is what makes a namespace or a
/// `template_declaration` transparent here without a dedicated case for either.
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
            "function_definition" => {
                let qualified = match function_declared_name(child, source) {
                    Some(DeclaredName::Segment(name)) => Some(join_path(prefix, name)),
                    Some(DeclaredName::QualifiedSuffix(suffix)) => Some(join_path(prefix, suffix)),
                    None => None,
                };
                if let (Some(qualified), Some(body)) =
                    (qualified.as_deref(), child.child_by_field_name("body"))
                {
                    collect_refs(body, source, prefix, Some(qualified), scope, out);
                }
            }
            "class_specifier" | "struct_specifier" => {
                let new_prefix = match field_text(child, "name", source) {
                    Some(name) => join_path(prefix, name),
                    None => prefix.to_string(),
                };
                if let Some(body) = child.child_by_field_name("body") {
                    collect_refs(body, source, &new_prefix, current_fn, scope, out);
                }
            }
            "namespace_definition" => {
                let new_prefix = match field_text(child, "name", source) {
                    Some(name) => join_path(prefix, name),
                    None => prefix.to_string(),
                };
                if let Some(body) = child.child_by_field_name("body") {
                    collect_refs(body, source, &new_prefix, current_fn, scope, out);
                }
            }
            "call_expression" => {
                if let (Some(from), Some(func)) =
                    (current_fn, child.child_by_field_name("function"))
                {
                    let (name, target) = call_target(func, source, scope);
                    if !name.is_empty() {
                        out.push(RefDecl {
                            from_qualified_path: from.to_string(),
                            to_name: name,
                            kind: EdgeKind::Calls,
                            to_target: target,
                        });
                    }
                }
                collect_refs(child, source, prefix, current_fn, scope, out);
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, scope, out);
            }
        }
    }
}
