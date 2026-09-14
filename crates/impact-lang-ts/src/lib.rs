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
//! (`extract_references`). An `interface`'s callable members — shorthand `method_signature`
//! (`foo(x): T`) and a `property_signature` typed with a function type (`foo: (x) => T`,
//! detected by inspecting its type annotation; a plain data property like `name: string`
//! isn't a symbol) — are indexed the same way a class's methods are (nested under the
//! interface's own name), so a call on a value typed only by an interface — never
//! implemented as a class in this file, e.g. a hand-typed binding to code this adapter
//! can't see into — still resolves through the same module+name matching a class method
//! would. A named function-scope isn't only `function foo() {}` —
//! `const foo = () => {}` and `const foo = function () {}` count too (see
//! `push_fn_valued_declarators`/`collect_refs_fn_valued_declarators`), since that's the
//! dominant style for React/React Native components and hooks; an unnamed arrow/function
//! expression (e.g. passed inline as a callback argument) still isn't a symbol of its own,
//! same as it never was. Detects tests by file-naming convention only (`is_test_file`)
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
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, FileAst, FileScope,
    LanguageAdapter, NodeKind, RefDecl, RefTarget, SymbolDecl,
};
use tree_sitter::Node;

mod tsconfig;
use tsconfig::PathAlias;

const HTTP_VERBS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

#[derive(Default)]
pub struct TsAdapter {
    config: DetectorConfig,
    path_aliases: Vec<PathAlias>,
}

impl TsAdapter {
    /// Reads `tsconfig.json` (falling back to `jsconfig.json`) at `project_root` for
    /// `compilerOptions.paths`/`baseUrl`, so imports written through a project's own
    /// module-alias convention (`@scope/*`, or a bare catch-all `"*"`) resolve the same
    /// way a relative import does. A missing or unparseable config file just means no
    /// aliases — never a hard error, since most projects have neither.
    pub fn new(config: DetectorConfig, project_root: &Path) -> Self {
        let path_aliases = tsconfig::load_path_aliases(project_root);
        Self {
            config,
            path_aliases,
        }
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
        mark_default_export(
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
        let scope = build_scope(
            ast.tree.root_node(),
            source,
            &ast.path,
            &prefix,
            &self.path_aliases,
        );
        let is_test_file = is_test_file(&ast.path);
        let mut out = Vec::new();
        collect_refs(
            ast.tree.root_node(),
            source,
            &prefix,
            None,
            is_test_file,
            &scope,
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

/// Whether an `interface_body` member is a callable worth indexing as a symbol: a
/// shorthand `method_signature` (`greet(x): T`) always is; a `property_signature` only is
/// when its own type annotation is itself a `function_type` (`farewell: (x) => T`) — a
/// plain data property (`name: string`) has no call-target to index and stays a non-event.
fn is_callable_interface_member(member: Node) -> bool {
    match member.kind() {
        "method_signature" => true,
        "property_signature" => member
            .child_by_field_name("type")
            .and_then(|annotation| annotation.named_child(0))
            .is_some_and(|inner| inner.kind() == "function_type"),
        _ => false,
    }
}

/// Finds this file's `export default ...` (there's at most one per module, by JS/TS's own
/// rules) and, if it names an identifier this file already declared as a symbol
/// (`export default XScreen` where `const XScreen = ...` or `function XScreen() {}` was
/// collected above), flags that one `SymbolDecl` with `is_default_export`. `export
/// default` is only valid at a file's top level, never nested, so this only needs to scan
/// `root`'s direct children rather than recursing — unlike `walk`, which already handles
/// an exported *declaration* (`export default function Foo() {}`, `export default class
/// Foo {}`) as an ordinary top-level declaration via its ordinary fallthrough, so by the
/// time this runs, the matching `SymbolDecl` already exists in `out` either way; this
/// pass only needs to find *which* one to flag. An anonymous default export (`export
/// default () => {}`, `export default someCall()`) introduces no name at all — nothing to
/// flag, same as it's never been a symbol of its own.
fn mark_default_export(root: Node, source: &[u8], prefix: &str, out: &mut [SymbolDecl]) {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "export_statement" {
            continue;
        }
        let Some(target) = default_export_target(child, source, prefix) else {
            continue;
        };
        if let Some(symbol) = out.iter_mut().find(|s| s.qualified_path == target) {
            symbol.is_default_export = true;
        }
        return;
    }
}

/// The qualified path `export default ...` in `export_statement` names, if it names
/// anything a symbol could exist under: a bare identifier (`export default XScreen`), or
/// a named `function`/`class` declaration (`export default function XScreen() {}`) —
/// `export`/`default` are anonymous keyword tokens (confirmed via a real parse-tree
/// dump), so the exported value is always the statement's one named child.
fn default_export_target(export_stmt: Node, source: &[u8], prefix: &str) -> Option<String> {
    let mut cursor = export_stmt.walk();
    let is_default = export_stmt
        .children(&mut cursor)
        .any(|c| c.kind() == "default");
    if !is_default {
        return None;
    }
    let value = export_stmt.named_child(0)?;
    let name = match value.kind() {
        "identifier" => value.utf8_text(source).ok()?,
        "function_declaration" | "class_declaration" => field_text(value, "name", source)?,
        _ => return None,
    };
    Some(join_path(prefix, name))
}

/// A JSX attribute's `{expr}` value, if `expr` is a bare identifier — `component={Foo}`,
/// not a string (`name="Foo"`), a call, or an inline arrow/function. `jsx_attribute` has
/// no named fields (confirmed via a real parse-tree dump), so the value is its second
/// named child when present at all (a boolean shorthand attribute like `<Foo disabled />`
/// has none).
fn jsx_attribute_value_identifier(attribute: Node) -> Option<Node> {
    let value = attribute.named_child(1)?;
    if value.kind() != "jsx_expression" {
        return None;
    }
    let inner = value.named_child(0)?;
    (inner.kind() == "identifier").then_some(inner)
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
        is_generated: false,
        is_default_export: false,
    });
}

/// Joins a `/`-separated relative spec (`./x`, `../y/z`, or an alias target like
/// `./src/packages/foo`) onto a starting segment list, the same way a filesystem `cd`
/// would: `.` is a no-op, `..` pops, anything else pushes. Shared by the relative-import
/// case (starting from the importer's own directory) and alias-target resolution
/// (starting from `baseUrl`).
pub(crate) fn join_relative<'a>(base: &[&'a str], spec: &'a str) -> String {
    let mut segments: Vec<&str> = base.to_vec();
    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    let joined = segments.join("/");
    [".tsx", ".ts", ".jsx", ".mjs", ".js"]
        .iter()
        .find_map(|ext| joined.strip_suffix(ext))
        .unwrap_or(&joined)
        .to_string()
}

/// Resolves an import specifier against the file doing the importing, producing the
/// module path the imported symbols are indexed under — `'./utils'` in
/// `src/store/sync/actions.js` becomes `store::sync::utils`.
///
/// A relative specifier resolves against the importing file's own directory. A bare one
/// (`'react'`, `'@newstore/foo'`, or a catch-all-aliased plain name) is tried against
/// `aliases` (from `tsconfig.json`'s `compilerOptions.paths`, see the `tsconfig` module)
/// next; a real package name simply won't match any configured pattern and stays
/// unresolved rather than being pinned to a guess.
///
/// Extensions are dropped rather than checked against disk, which also makes this
/// indifferent to whether `./utils` is `utils.ts`, `utils.js` or `utils/index.ts` — the
/// linker matches on module segments, and `utils/index.ts` is indexed under a path that
/// still contains `utils` (see `Resolver::in_module`).
fn resolve_specifier(
    importer_path: &str,
    specifier: &str,
    aliases: &[PathAlias],
) -> Option<String> {
    if specifier.starts_with('.') {
        let importer = importer_path.replace('\\', "/");
        let mut base: Vec<&str> = importer.split('/').collect();
        base.pop();
        return Some(module_prefix(&join_relative(&base, specifier)));
    }
    tsconfig::resolve_alias(specifier, aliases).map(|joined| module_prefix(&joined))
}

/// Reads this file's imports and its own top-level declarations into a `FileScope`.
///
/// Covers the shapes real TypeScript and JavaScript actually use: named, default and
/// namespace `import`s, `export ... from` re-exports, and `require()` bound to a `const`
/// (destructured or whole). A name that none of them introduce, and that this file
/// doesn't declare, is `Opaque` — in a module system where cross-file access requires an
/// import, its absence is real information.
fn build_scope(
    root: Node,
    source: &[u8],
    file_path: &str,
    prefix: &str,
    aliases: &[PathAlias],
) -> FileScope {
    let mut scope = FileScope::new(prefix, RefTarget::Opaque);
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" | "export_statement" => {
                collect_import(child, source, file_path, aliases, &mut scope);
                // `export function foo() {}` declares `foo` here as much as a bare
                // declaration does.
                declare_locals(child, source, &mut scope);
            }
            "lexical_declaration" | "variable_declaration" => {
                collect_require(child, source, file_path, aliases, &mut scope);
                declare_locals(child, source, &mut scope);
            }
            _ => declare_locals(child, source, &mut scope),
        }
    }
    scope
}

/// Records every name an `import`/`export ... from` statement binds, against the module
/// its specifier resolves to.
fn collect_import(
    statement: Node,
    source: &[u8],
    file_path: &str,
    aliases: &[PathAlias],
    scope: &mut FileScope,
) {
    let Some(specifier) = statement
        .child_by_field_name("source")
        .and_then(|s| ts_string_text(s, source))
    else {
        return;
    };
    let Some(module) = resolve_specifier(file_path, &specifier, aliases) else {
        return;
    };

    let mut cursor = statement.walk();
    for child in statement.children(&mut cursor) {
        match child.kind() {
            // `import Default from './x'` — see `bind_import_clause`'s "identifier" arm
            // for why this binds via `add_default_import`, not `add_import`.
            "import_clause" => bind_import_clause(child, source, &module, scope),
            // `export { a, b } from './x'` — the re-exporting file is a path to those
            // symbols too, so a caller importing them from here still lands in `./x`.
            "export_clause" => bind_named_imports(child, source, &module, scope),
            _ => {}
        }
    }
}

fn bind_import_clause(clause: Node, source: &[u8], module: &str, scope: &mut FileScope) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            // `import Foo from './x'` — a *default* import. The local name (`Foo`) is
            // chosen by this file, not by whatever `./x` actually calls its own default
            // export (see `is_default_export`/`mark_default_exports`), so binding it via
            // `add_default_import` matters: it's the difference between a reference
            // resolving only when the importer happens to reuse the export's own name,
            // and resolving whenever the target module has exactly one default export at
            // all, the same way `export default XScreen` imported as `import Foo from
            // './XScreen'` actually works in real code.
            "identifier" => {
                if let Ok(name) = child.utf8_text(source) {
                    scope.add_default_import(name, module);
                }
            }
            "named_imports" => bind_named_imports(child, source, module, scope),
            "namespace_import" => {
                // `import * as helpers from './x'` — `helpers.foo()` is a call into `./x`.
                let mut inner = child.walk();
                for namespace in child.children(&mut inner) {
                    if namespace.kind() == "identifier" {
                        if let Ok(name) = namespace.utf8_text(source) {
                            scope.add_import(name, module);
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Binds each specifier in a `{ a, b as c }` list, keying on the local name (`c`), since
/// that's what call sites in this file write.
fn bind_named_imports(list: Node, source: &[u8], module: &str, scope: &mut FileScope) {
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if !matches!(child.kind(), "import_specifier" | "export_specifier") {
            continue;
        }
        let local = child
            .child_by_field_name("alias")
            .or_else(|| child.child_by_field_name("name"));
        if let Some(name) = local.and_then(|n| n.utf8_text(source).ok()) {
            scope.add_import(name, module);
        }
    }
}

/// `const { a, b } = require('./x')` and `const x = require('./x')` — CommonJS, still the
/// shape a lot of real JavaScript uses.
fn collect_require(
    declaration: Node,
    source: &[u8],
    file_path: &str,
    aliases: &[PathAlias],
    scope: &mut FileScope,
) {
    let mut cursor = declaration.walk();
    for declarator in declaration.children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let Some(module) = declarator
            .child_by_field_name("value")
            .and_then(|value| require_specifier(value, source))
            .and_then(|specifier| resolve_specifier(file_path, &specifier, aliases))
        else {
            continue;
        };
        let Some(pattern) = declarator.child_by_field_name("name") else {
            continue;
        };
        match pattern.kind() {
            "identifier" => {
                if let Ok(name) = pattern.utf8_text(source) {
                    scope.add_import(name, &module);
                }
            }
            "object_pattern" => {
                let mut inner = pattern.walk();
                for element in pattern.children(&mut inner) {
                    let name = match element.kind() {
                        "shorthand_property_identifier_pattern" => element.utf8_text(source).ok(),
                        "pair_pattern" => element
                            .child_by_field_name("value")
                            .and_then(|v| v.utf8_text(source).ok()),
                        _ => None,
                    };
                    if let Some(name) = name {
                        scope.add_import(name, &module);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The string argument of a `require(...)` call, or `None` if this isn't one.
fn require_specifier(value: Node, source: &[u8]) -> Option<String> {
    if value.kind() != "call_expression" {
        return None;
    }
    let callee = value.child_by_field_name("function")?;
    if callee.utf8_text(source).ok()? != "require" {
        return None;
    }
    let arguments = value.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let mut found = None;
    for argument in arguments.children(&mut cursor) {
        if let Some(text) = ts_string_text(argument, source) {
            found = Some(text);
            break;
        }
    }
    found
}

/// Records the names a top-level declaration introduces, so a call to one of them can be
/// tied to this file rather than to a same-named symbol elsewhere.
fn declare_locals(node: Node, source: &[u8], scope: &mut FileScope) {
    match node.kind() {
        "function_declaration" | "class_declaration" => {
            if let Some(name) = field_text(node, "name", source) {
                scope.declare_local(name);
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for declarator in node.children(&mut cursor) {
                if declarator.kind() != "variable_declarator" {
                    continue;
                }
                if let Some(name) = field_text(declarator, "name", source) {
                    scope.declare_local(name);
                }
            }
        }
        "export_statement" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                declare_locals(child, source, scope);
            }
        }
        _ => {}
    }
}

/// Where a call's callee points, as far as this file can say: a bare name it imports or
/// declares, `this.method()` (the enclosing class is declared here), or a call through an
/// imported namespace. A method call on anything else has a receiver this adapter can't
/// type, so it stays `Opaque`.
fn call_target(callee: Node, source: &[u8], scope: &FileScope) -> RefTarget {
    match callee.kind() {
        "identifier" => callee
            .utf8_text(source)
            .map(|name| scope.bare(name))
            .unwrap_or(RefTarget::Opaque),
        "member_expression" => match callee.child_by_field_name("object") {
            Some(object) if object.kind() == "this" => scope.own(),
            Some(object) if object.kind() == "identifier" => object
                .utf8_text(source)
                .map(|name| scope.qualified(name))
                .unwrap_or(RefTarget::Opaque),
            _ => RefTarget::Opaque,
        },
        _ => RefTarget::Opaque,
    }
}

/// The call names that introduce a named scope in a test file. Jest, Vitest and Mocha all
/// share these, which is the same reasoning `is_test_file` uses: a convention all three
/// agree on is a convention worth reading, unlike the parts where they differ.
///
/// Lifecycle hooks (`beforeEach` and friends) are deliberately absent. They aren't tests
/// and shouldn't be counted as any, and a hook body belongs to whichever block encloses
/// it — which is what happens anyway, since a call's arguments are walked with the
/// enclosing scope unchanged.
const TEST_BLOCK_NAMES: &[&str] = &["describe", "context", "suite", "it", "test", "specify"];

/// The scope name a `describe(...)`/`it(...)` call introduces, together with the callback
/// whose body belongs to it — or `None` if this isn't a test block.
///
/// Real suites put every assertion inside an anonymous callback, which introduces no
/// named function and so used to swallow every call in it: a JS/TS project's tests were
/// invisible to this tool entirely. Naming the block after its own title is what makes a
/// reported test something a developer can act on ("run this one") rather than a file
/// reference.
fn test_block<'a>(call: Node<'a>, source: &[u8]) -> Option<(String, Node<'a>)> {
    let callee = call.child_by_field_name("function")?;
    let runner = match callee.kind() {
        "identifier" => callee.utf8_text(source).ok()?,
        // `describe.only(...)`, `it.skip(...)`, `test.concurrent(...)`.
        "member_expression" => callee
            .child_by_field_name("object")
            .filter(|object| object.kind() == "identifier")
            .and_then(|object| object.utf8_text(source).ok())?,
        _ => return None,
    };
    if !TEST_BLOCK_NAMES.contains(&runner) {
        return None;
    }

    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let mut title = None;
    let mut body = None;
    for argument in arguments.children(&mut cursor) {
        match argument.kind() {
            "string" if title.is_none() => title = ts_string_text(argument, source),
            "arrow_function" | "function_expression" if body.is_none() => body = Some(argument),
            _ => {}
        }
    }

    // A title that isn't a plain literal (a template string, a variable, a `.each` table)
    // can't be read structurally, so the block falls back to the runner's own name. Two
    // such siblings collapse into one scope, which is a coarser answer than usual but
    // still attributes their calls to the right file and suite.
    let title = title.unwrap_or_else(|| runner.to_string());
    Some((sanitize_block_title(&title), body?))
}

/// Keeps a block title from inventing path segments: `::` is how qualified paths are
/// joined, so a title containing it would parse as nesting that isn't there.
fn sanitize_block_title(title: &str) -> String {
    title.replace("::", ":")
}

/// Walks declarations at any depth (transparently unwrapping `export`/`export default`),
/// extracting one `SymbolDecl` per function, class, method, and interface member —
/// including a named function/arrow/expression declared *inside* another function's body
/// (a local helper closure, e.g. `function outer() { const helper = () => {...} }`).
/// `collect_refs` already recurses into every function body unconditionally and attributes
/// a nested named helper's own calls to a flat `prefix::helper` qualified path (never truly
/// nested through intermediate scopes, regardless of depth) — this walk matches that same
/// scheme, registering each such helper under the *same* `prefix` it was called with rather
/// than a nested one, so the two passes agree on what `helper`'s qualified path is. Without
/// this, `collect_refs` names a caller that `extract_symbols` never created, and the
/// linker's `from`-side lookup fails, silently dropping the whole call — regardless of
/// whether the callee resolves. `class_declaration`/`interface_declaration` bodies are the
/// one case with a genuinely nested prefix (the class/interface name), handled by
/// recursing explicitly with `new_prefix` instead of falling into the generic recursion
/// below. `is_test_file` marks every function/method `is_test` (never a class itself) —
/// see `is_test_file`'s own doc for the convention; a test file's `describe`/`it` blocks
/// are handled entirely by `walk_test_blocks` instead (its own full recursion), so this
/// walk doesn't also descend into them generically once it hands off to that.
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
                continue;
            }
            "method_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Function, prefix, name, child, is_test_file);
                }
            }
            "interface_declaration" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if let Some(body) = child.child_by_field_name("body") {
                        let new_prefix = join_path(prefix, name);
                        let mut inner = body.walk();
                        for member in body.children(&mut inner) {
                            if is_callable_interface_member(member) {
                                if let Some(method_name) = field_text(member, "name", source) {
                                    push(
                                        out,
                                        NodeKind::Function,
                                        &new_prefix,
                                        method_name,
                                        member,
                                        false,
                                    );
                                }
                            }
                        }
                    }
                }
                continue;
            }
            "lexical_declaration" | "variable_declaration" => {
                push_fn_valued_declarators(child, source, prefix, is_test_file, out);
                push_call_valued_declarators(child, source, prefix, is_test_file, out);
            }
            _ if is_test_file => {
                walk_test_blocks(child, source, prefix, out);
                continue;
            }
            _ => {}
        }
        // Keep descending with the same `prefix` regardless of what (if anything) this
        // child was — this is what reaches a named helper nested inside a function body,
        // an `if`/`try` block, or any other depth, instead of stopping after one level.
        // `export_statement` (`export function foo() {}`) needs no special unwrap arm any
        // more: it isn't matched above, so it falls straight through to this same
        // recursion, same as it would if we'd special-cased it.
        walk(child, source, prefix, is_test_file, out);
    }
}

/// Finds `describe`/`it` blocks at any depth in a test file and registers each as a
/// symbol named after its own title, nested under whichever blocks enclose it.
///
/// Separate from `walk` because it wants the opposite thing: `walk` deliberately stops at
/// a function body, while a suite's structure *is* nested function bodies, and the tests
/// worth naming are all inside them.
fn walk_test_blocks(node: Node, source: &[u8], prefix: &str, out: &mut Vec<SymbolDecl>) {
    if node.kind() == "call_expression" {
        if let Some((title, body)) = test_block(node, source) {
            push(out, NodeKind::Function, prefix, &title, node, true);
            let new_prefix = join_path(prefix, &title);
            walk_test_blocks(body, source, &new_prefix, out);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_test_blocks(child, source, prefix, out);
    }
}

/// A `const`/`let`/`var` declaration's `variable_declarator` children whose value is an
/// `arrow_function` or `function_expression` are named function-scopes too —
/// `const Foo = () => {...}` and `const Foo = function () {...}` are the dominant style
/// for React/React Native components and hooks, and were previously invisible to this
/// adapter entirely (see the module doc for the false-negative this fixes). Anything else
/// (`const x = 5`, `const { a, b } = y`) isn't a function and is skipped.
fn push_fn_valued_declarators(
    decl: Node,
    source: &[u8],
    prefix: &str,
    is_test_file: bool,
    out: &mut Vec<SymbolDecl>,
) {
    let mut cursor = decl.walk();
    for declarator in decl.children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let is_fn_value = declarator
            .child_by_field_name("value")
            .is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression"));
        if !is_fn_value {
            continue;
        }
        if let Some(name) = field_text(declarator, "name", source) {
            push(
                out,
                NodeKind::Function,
                prefix,
                name,
                declarator,
                is_test_file,
            );
        }
    }
}

/// A top-level `const`/`let`/`var` whose initializer *calls* something runs that call at
/// module load, which makes the binding a call site with a name — the only name there is
/// to attribute the call to, since no function encloses it. Registering it as a symbol is
/// what keeps `export const CLIENT = createClient(config)` from vanishing out of
/// `config`'s blast radius.
///
/// A binding whose initializer calls nothing (`const COLORS = ['red']`) isn't a call site
/// and isn't indexed — this is about not losing edges, not about cataloguing constants.
/// Function-valued declarators are already handled by `push_fn_valued_declarators`.
fn push_call_valued_declarators(
    decl: Node,
    source: &[u8],
    prefix: &str,
    is_test_file: bool,
    out: &mut Vec<SymbolDecl>,
) {
    let mut cursor = decl.walk();
    for declarator in decl.children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let Some(value) = declarator.child_by_field_name("value") else {
            continue;
        };
        if matches!(value.kind(), "arrow_function" | "function_expression") {
            continue;
        }
        if !contains_call(value) {
            continue;
        }
        if let Some(name) = field_text(declarator, "name", source) {
            push(out, NodeKind::Field, prefix, name, declarator, is_test_file);
        }
    }
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
#[allow(clippy::too_many_arguments)]
fn collect_refs(
    node: Node,
    source: &[u8],
    prefix: &str,
    current_fn: Option<&str>,
    is_test_file: bool,
    scope: &FileScope,
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "method_definition" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_refs(
                            body,
                            source,
                            prefix,
                            Some(&qualified),
                            is_test_file,
                            scope,
                            out,
                        );
                    }
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                collect_refs_fn_valued_declarators(
                    child,
                    source,
                    prefix,
                    current_fn,
                    is_test_file,
                    scope,
                    out,
                );
            }
            "call_expression" if is_test_file && test_block(child, source).is_some() => {
                // Safe to unwrap the option we just matched on; kept as a `let` so the
                // borrow of `child` lives long enough for the recursive walk.
                if let Some((title, body)) = test_block(child, source) {
                    let qualified = join_path(prefix, &title);
                    collect_refs(
                        body,
                        source,
                        &join_path(prefix, &title),
                        Some(&qualified),
                        is_test_file,
                        scope,
                        out,
                    );
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
                            to_target: call_target(func, source, scope),
                        });
                    }
                }
                // A chained call puts its receiver in the callee, not the arguments:
                // `expect(value).toEqual(x)` and `getUser().save()` both hide a whole
                // call in there, and descending only into arguments dropped it.
                if let Some(callee) = child.child_by_field_name("function") {
                    collect_refs(callee, source, prefix, current_fn, is_test_file, scope, out);
                }
                if let Some(args) = child.child_by_field_name("arguments") {
                    collect_refs(args, source, prefix, current_fn, is_test_file, scope, out);
                }
            }
            "class_declaration" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_refs(
                        body,
                        source,
                        &new_prefix,
                        current_fn,
                        is_test_file,
                        scope,
                        out,
                    );
                }
            }
            // A bare identifier handed somewhere as *data* rather than called — a JSX
            // attribute's expression value (`component={Foo}`, the shape a React
            // Navigation-style route registry uses) or an object-literal property's value
            // (`{ screen: Foo }`) — is a real dependency edge `call_expression`-only
            // extraction always missed. Emitted as `References` (already used by
            // impact-lang-rust's enum-variant references, already unioned into
            // blast-radius traversal), resolved through the same `scope` a call-target
            // identifier would be, so it gets the same confidence tiering.
            "jsx_attribute" => {
                if let Some(from) = current_fn {
                    if let Some(identifier) = jsx_attribute_value_identifier(child) {
                        if let Ok(name) = identifier.utf8_text(source) {
                            out.push(RefDecl {
                                from_qualified_path: from.to_string(),
                                to_name: name.to_string(),
                                kind: EdgeKind::References,
                                to_target: scope.bare(name),
                            });
                        }
                    }
                }
                collect_refs(child, source, prefix, current_fn, is_test_file, scope, out);
            }
            "pair" => {
                if let Some(from) = current_fn {
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "identifier" {
                            if let Ok(name) = value.utf8_text(source) {
                                out.push(RefDecl {
                                    from_qualified_path: from.to_string(),
                                    to_name: name.to_string(),
                                    kind: EdgeKind::References,
                                    to_target: scope.bare(name),
                                });
                            }
                        }
                    }
                }
                collect_refs(child, source, prefix, current_fn, is_test_file, scope, out);
            }
            // A leaf node (`{ Foo }`, shorthand for `{ Foo: Foo }`) — no children to
            // recurse into.
            "shorthand_property_identifier" => {
                if let Some(from) = current_fn {
                    if let Ok(name) = child.utf8_text(source) {
                        out.push(RefDecl {
                            from_qualified_path: from.to_string(),
                            to_name: name.to_string(),
                            kind: EdgeKind::References,
                            to_target: scope.bare(name),
                        });
                    }
                }
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, is_test_file, scope, out);
            }
        }
    }
}

/// The `collect_refs` counterpart to `push_fn_valued_declarators`: for each
/// `variable_declarator` in a `const`/`let`/`var` declaration whose value is an
/// `arrow_function` or `function_expression`, recurse into that function with `current_fn`
/// switched to the declarator's name — mirroring the `function_declaration` arm above.
/// Recursing from the function node itself (not just its `body` field) matters for a
/// concise/expression-bodied arrow function like `() => helper()`: there, tree-sitter puts
/// the call expression directly in the `body` field rather than inside a
/// `statement_block`, so descending from the function node lets the normal per-child
/// dispatch below (the `call_expression` arm) match it either way. Any other declarator
/// (not function-valued, e.g. `const data = fetchData();`) isn't a new function scope, so
/// it recurses with `current_fn` unchanged — same as today's default fallthrough.
#[allow(clippy::too_many_arguments)]
fn collect_refs_fn_valued_declarators(
    decl: Node,
    source: &[u8],
    prefix: &str,
    current_fn: Option<&str>,
    is_test_file: bool,
    scope: &FileScope,
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = decl.walk();
    for declarator in decl.children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let fn_value = declarator
            .child_by_field_name("value")
            .filter(|v| matches!(v.kind(), "arrow_function" | "function_expression"));
        match (field_text(declarator, "name", source), fn_value) {
            (Some(name), Some(value)) => {
                let qualified = join_path(prefix, name);
                collect_refs(
                    value,
                    source,
                    prefix,
                    Some(&qualified),
                    is_test_file,
                    scope,
                    out,
                );
            }
            // Not a function, but its initializer may still run a call — and at the top
            // level (`current_fn` is `None`) the binding is the only thing that call can
            // be attributed to. Inside a function the enclosing function is the better
            // answer, so nothing changes there.
            (Some(name), None) if current_fn.is_none() && contains_call(declarator) => {
                let qualified = join_path(prefix, name);
                collect_refs(
                    declarator,
                    source,
                    prefix,
                    Some(&qualified),
                    is_test_file,
                    scope,
                    out,
                );
            }
            _ => collect_refs(
                declarator,
                source,
                prefix,
                current_fn,
                is_test_file,
                scope,
                out,
            ),
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
