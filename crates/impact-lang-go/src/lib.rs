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

use std::collections::HashMap;
use std::path::Path;

use impact_core::{
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, FileAst, FileScope,
    LanguageAdapter, NodeKind, RefDecl, RefTarget, SymbolDecl,
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
        let is_generated_file = is_generated_file(&ast.source);
        let mut out = Vec::new();
        walk(
            ast.tree.root_node(),
            ast.source.as_bytes(),
            &prefix,
            is_test_file,
            is_generated_file,
            &mut out,
        );
        out
    }

    fn extract_references(&self, ast: &FileAst) -> Vec<RefDecl> {
        let prefix = module_prefix(&ast.path);
        let source = ast.source.as_bytes();
        let scope = build_scope(ast.tree.root_node(), source, &prefix);
        let types = collect_declared_types(ast.tree.root_node(), source);
        let struct_fields = collect_struct_fields(ast.tree.root_node(), source);
        let ctx = FileContext {
            scope,
            types,
            struct_fields,
        };
        let mut out = Vec::new();
        collect_refs(ast.tree.root_node(), source, &prefix, None, &ctx, &mut out);
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
    is_generated: bool,
) {
    out.push(SymbolDecl {
        kind,
        qualified_path,
        line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        is_test,
        is_generated,
        is_default_export: false,
    });
}

/// Whether this file carries Go's own standard "this file was generated by a tool, don't
/// hand-edit it" marker (https://go.dev/s/generatedcode — `mockgen`, `protoc-gen-go`,
/// `stringer`, `wire`, and the rest of the ecosystem all emit exactly this line): a
/// comment matching `^// Code generated .* DO NOT EDIT\.$` on its own line, anywhere in
/// the file. Structural, not a guess — this is the one signal the Go toolchain itself
/// (`cmd/go`, `goimports`, ...) already treats as authoritative for "generated file".
fn is_generated_file(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_end_matches('\r');
        line.starts_with("// Code generated ") && line.ends_with(" DO NOT EDIT.")
    })
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
fn walk(
    node: Node,
    source: &[u8],
    prefix: &str,
    is_test_file: bool,
    is_generated_file: bool,
    out: &mut Vec<SymbolDecl>,
) {
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
                        is_generated_file,
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
                        push(
                            out,
                            NodeKind::Function,
                            qualified,
                            child,
                            false,
                            is_generated_file,
                        );
                    }
                }
            }
            "type_declaration" => {
                if let Some(name) = type_declaration_name(child, source) {
                    push(
                        out,
                        NodeKind::Type,
                        join_path(prefix, name),
                        child,
                        false,
                        is_generated_file,
                    );
                }
            }
            "var_declaration" | "const_declaration" => {
                for (name, spec) in package_level_bindings(child, source) {
                    push(
                        out,
                        NodeKind::Field,
                        join_path(prefix, name),
                        spec,
                        false,
                        is_generated_file,
                    );
                }
            }
            _ => {}
        }
    }
}

/// Package-level `var`/`const` declarations whose initializer *calls* something. That
/// call runs at package initialization, which makes the binding a call site with a name —
/// the only name there is to attribute it to, since no function encloses it. Without
/// this, `var db = mustConnect()` disappeared from `mustConnect`'s blast radius.
///
/// A binding that calls nothing (`var limit = 10`) isn't a call site and isn't indexed.
fn package_level_bindings<'a>(declaration: Node<'a>, source: &'a [u8]) -> Vec<(&'a str, Node<'a>)> {
    let mut out = Vec::new();
    let mut cursor = declaration.walk();
    for spec in declaration.children(&mut cursor) {
        if !matches!(spec.kind(), "var_spec" | "const_spec") {
            continue;
        }
        let Some(value) = spec.child_by_field_name("value") else {
            continue;
        };
        if !contains_call(value) {
            continue;
        }
        if let Some(name) = field_text(spec, "name", source) {
            out.push((name, spec));
        }
    }
    out
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

/// Walks the same top-level shapes as `walk`, but descends into function/method bodies
/// (which `walk` deliberately doesn't) to find `call_expression`s, recording each as a
/// `RefDecl` from the enclosing function. `current_fn` is `None` outside any function
/// body, matching every other adapter's `collect_refs`.
/// This file's imports and declarations, plus the declared type of every named value in
/// it. Go states types everywhere they matter — method receivers, parameters, `var`
/// declarations — so a method call's receiver almost always has a type to look up, with
/// no inference involved.
///
/// `struct_fields` extends the same idea one level in: idiomatic Go dependency injection
/// calls through a struct field (`uc.ChangesRepository.AddOperation(...)`) rather than the
/// receiver directly, so resolving that call needs the field's own declared type, not just
/// the receiver's. Scoped to this file only, like `types` — a struct declared in a sibling
/// file of the same package won't be found (see `collect_struct_fields`).
struct FileContext {
    scope: FileScope,
    types: HashMap<String, String>,
    struct_fields: HashMap<String, HashMap<String, String>>,
}

/// The directory a file's package lives in — its module path minus the filename. Go
/// scopes by package, i.e. by directory, so a name with no import behind it is most
/// likely a sibling file in that same directory rather than something unresolvable.
fn package_module(prefix: &str) -> String {
    let mut segments: Vec<&str> = prefix.split("::").filter(|s| !s.is_empty()).collect();
    segments.pop();
    segments.join("::")
}

fn build_scope(root: Node, source: &[u8], prefix: &str) -> FileScope {
    let package = package_module(prefix);
    let mut scope = FileScope::new(prefix, RefTarget::Module(package));
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => collect_imports(child, source, &mut scope),
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    scope.declare_local(name);
                }
            }
            "type_declaration" => {
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if spec.kind() == "type_spec" {
                        if let Some(name) = field_text(spec, "name", source) {
                            scope.declare_local(name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    scope
}

/// Binds each import's package name — its alias, or the last segment of its path — to
/// that last segment, which is the directory the package's files live in and therefore
/// the module its symbols are indexed under.
fn collect_imports(declaration: Node, source: &[u8], scope: &mut FileScope) {
    collect_import_specs(declaration, source, scope);
}

fn collect_import_specs(node: Node, source: &[u8], scope: &mut FileScope) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec" {
            let Some(path) = child
                .child_by_field_name("path")
                .and_then(|p| string_literal_text(p, source))
            else {
                continue;
            };
            let Some(directory) = path.rsplit('/').next().filter(|s| !s.is_empty()) else {
                continue;
            };
            let alias = field_text(child, "name", source).unwrap_or(directory);
            scope.add_import(alias, directory);
            continue;
        }
        collect_import_specs(child, source, scope);
    }
}

/// Maps every named value in this file to its declared type: method receivers, function
/// parameters, and `var` declarations. Keyed by name across the file — two functions
/// disagreeing on what `s` is costs a confidence tier, not a missed caller.
fn collect_declared_types(root: Node, source: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    collect_declared_types_inner(root, source, &mut out);
    out
}

fn collect_declared_types_inner(node: Node, source: &[u8], out: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "parameter_declaration" | "var_spec") {
            if let (Some(name), Some(type_node)) = (
                field_text(child, "name", source),
                child.child_by_field_name("type"),
            ) {
                if let Ok(text) = type_node.utf8_text(source) {
                    out.insert(
                        name.to_string(),
                        text.trim_start_matches(['*', '&']).to_string(),
                    );
                }
            }
        }
        collect_declared_types_inner(child, source, out);
    }
}

/// Maps every struct type declared in this file to its field names' declared types
/// (stripped of a leading `*`, matching `collect_declared_types`) — the piece idiomatic Go
/// dependency injection needs: a use-case struct's field is typed as an interface
/// (`ChangesRepository changes.Repository`), and a call through it
/// (`uc.ChangesRepository.AddOperation(...)`) can only resolve once that field's own
/// declared type is known, not just `uc`'s.
///
/// Grammar confirmed via a real parse-tree dump before writing this: a `type_declaration`
/// whose `type_spec` has a `struct_type` body holds an unnamed (positional, no field label
/// in the grammar) `field_declaration_list` child, itself holding `field_declaration`
/// nodes with named `name`/`type` fields — `type` covers `type_identifier` (a local type),
/// `qualified_type` (`pkg.Type`), and `pointer_type` alike, since `.utf8_text()` returns
/// each one's own literal source text regardless of node kind.
fn collect_struct_fields(root: Node, source: &[u8]) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "type_declaration" {
            continue;
        }
        let mut spec_cursor = child.walk();
        for spec in child.children(&mut spec_cursor) {
            if spec.kind() != "type_spec" {
                continue;
            }
            let Some(type_name) = field_text(spec, "name", source) else {
                continue;
            };
            let Some(struct_type) = spec
                .child_by_field_name("type")
                .filter(|t| t.kind() == "struct_type")
            else {
                continue;
            };
            let mut struct_cursor = struct_type.walk();
            let Some(field_list) = struct_type
                .children(&mut struct_cursor)
                .find(|c| c.kind() == "field_declaration_list")
            else {
                continue;
            };
            let mut fields = HashMap::new();
            let mut field_cursor = field_list.walk();
            for field_decl in field_list.children(&mut field_cursor) {
                if field_decl.kind() != "field_declaration" {
                    continue;
                }
                if let (Some(field_name), Some(field_type)) = (
                    field_text(field_decl, "name", source),
                    field_decl.child_by_field_name("type"),
                ) {
                    if let Ok(text) = field_type.utf8_text(source) {
                        fields.insert(
                            field_name.to_string(),
                            text.trim_start_matches(['*', '&']).to_string(),
                        );
                    }
                }
            }
            out.insert(type_name.to_string(), fields);
        }
    }
    out
}

/// Where a call's callee points. A bare name is this package unless an import shadows it;
/// a selector (`pkg.Func()`, `p.Method()`, or a field-selector chain like
/// `uc.Field.Method()`) resolves through whichever the operand is — an imported package, a
/// value whose declared type this file states, or (recursively) a field of one.
fn call_target(callee: Node, source: &[u8], ctx: &FileContext) -> RefTarget {
    match callee.kind() {
        "identifier" => callee
            .utf8_text(source)
            .map(|name| ctx.scope.bare(name))
            .unwrap_or(RefTarget::Opaque),
        "selector_expression" => match callee.child_by_field_name("operand") {
            Some(operand) => selector_operand_target(operand, source, ctx),
            None => RefTarget::Opaque,
        },
        _ => RefTarget::Opaque,
    }
}

/// Resolves a selector's operand *node* — not its flat source text, which is what made a
/// field-selector chain invisible before this: for a nested selector
/// (`uc.ChangesRepository` inside `uc.ChangesRepository.AddOperation()`),
/// `.utf8_text()` on the whole operand would hand `operand_target` the literal string
/// `"uc.ChangesRepository"`, which isn't a name in scope at all (not an import, not a
/// declared type) — structurally wrong, since `operand_target`/`ctx.scope.qualified`
/// expect a single name, not a dotted expression. Confirmed via a real dogfooding session:
/// this is Go's dominant dependency-injection idiom (a struct field typed as an interface,
/// called through one level of field access), and it accounted for real call sites this
/// adapter was silently missing.
///
/// A plain identifier operand resolves exactly as before (`operand_target`); a selector
/// operand resolves via `expr_declared_type`, which walks as many field hops as this file's
/// own struct declarations can answer — one hop covers the confirmed real-world case, and
/// the recursion costs nothing extra to also cover more.
fn selector_operand_target(operand: Node, source: &[u8], ctx: &FileContext) -> RefTarget {
    match operand.kind() {
        "identifier" => operand
            .utf8_text(source)
            .map(|name| operand_target(name, ctx))
            .unwrap_or(RefTarget::Opaque),
        "selector_expression" => match expr_declared_type(operand, source, ctx) {
            Some(declared) => resolve_declared_type(&declared, ctx),
            None => RefTarget::Opaque,
        },
        _ => RefTarget::Opaque,
    }
}

/// A selector's operand is either an imported package name or a value. A value's declared
/// type is what says where its methods live — and a qualified type (`svc.Service`) says
/// which package, via that package's own import.
fn operand_target(operand: &str, ctx: &FileContext) -> RefTarget {
    let Some(declared) = ctx.types.get(operand) else {
        return ctx.scope.qualified(operand);
    };
    resolve_declared_type(declared, ctx)
}

/// Resolves an already-known declared type name (a var/param/receiver type, or a struct
/// field's type — stripped of a leading `*` in both cases) to the module its methods live
/// in: a package-qualified type (`svc.Service`) resolves through that package's own
/// import; a bare type name resolves as a local declaration. Shared by `operand_target` and
/// `selector_operand_target` so the "what does this type name mean" question is answered
/// once.
fn resolve_declared_type(declared: &str, ctx: &FileContext) -> RefTarget {
    match declared.split_once('.') {
        Some((package, _)) => ctx.scope.qualified(package),
        None => ctx.scope.qualified(declared),
    }
}

/// The declared type of an expression node this adapter can type-resolve at all: an
/// identifier's own declared var/param/receiver type (`ctx.types`), or, recursively, a
/// field-selector chain's final field type — each hop looks up the previous hop's declared
/// type's struct fields (`ctx.struct_fields`, itself scoped to this file's own
/// declarations). `None` for anything else (a call result, an index expression, a struct
/// declared outside this file, ...) — those call sites stay unresolved rather than guessed
/// at, the same tradeoff every other "no evidence, no `Exact`" case in this adapter makes.
fn expr_declared_type(expr: Node, source: &[u8], ctx: &FileContext) -> Option<String> {
    match expr.kind() {
        "identifier" => {
            let name = expr.utf8_text(source).ok()?;
            ctx.types.get(name).cloned()
        }
        "selector_expression" => {
            let receiver = expr.child_by_field_name("operand")?;
            let field = expr.child_by_field_name("field")?.utf8_text(source).ok()?;
            let receiver_type = expr_declared_type(receiver, source, ctx)?;
            let struct_name = receiver_type.rsplit('.').next().unwrap_or(&receiver_type);
            ctx.struct_fields.get(struct_name)?.get(field).cloned()
        }
        _ => None,
    }
}

fn collect_refs(
    node: Node,
    source: &[u8],
    prefix: &str,
    current_fn: Option<&str>,
    ctx: &FileContext,
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_refs(body, source, prefix, Some(&qualified), ctx, out);
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
                            collect_refs(body, source, prefix, Some(&qualified), ctx, out);
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
                            to_target: call_target(func, source, ctx),
                        });
                    }
                }
                // A chained call hides a whole call in its callee rather than its
                // arguments: `New().Charge()` calls `New` too.
                if let Some(callee) = child.child_by_field_name("function") {
                    collect_refs(callee, source, prefix, current_fn, ctx, out);
                }
                if let Some(args) = child.child_by_field_name("arguments") {
                    collect_refs(args, source, prefix, current_fn, ctx, out);
                }
            }
            // At package level (`current_fn` is `None`) a binding's initializer is the
            // only named thing its calls can belong to.
            "var_declaration" | "const_declaration" if current_fn.is_none() => {
                for (name, spec) in package_level_bindings(child, source) {
                    let qualified = join_path(prefix, name);
                    collect_refs(spec, source, prefix, Some(&qualified), ctx, out);
                }
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, ctx, out);
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
