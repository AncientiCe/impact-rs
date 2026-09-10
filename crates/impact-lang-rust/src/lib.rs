use std::collections::HashMap;
use std::path::Path;

use impact_core::{
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, EventStrategy, FileAst,
    FileScope, LanguageAdapter, NodeKind, RefDecl, RefTarget, SymbolDecl,
};
use tree_sitter::Node;

pub struct RustAdapter {
    config: DetectorConfig,
}

impl RustAdapter {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

    fn language() -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }
}

impl Default for RustAdapter {
    fn default() -> Self {
        Self::new(DetectorConfig::default())
    }
}

impl LanguageAdapter for RustAdapter {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.rs"]
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
            &self.config,
            &mut out,
        );
        out
    }

    fn extract_references(&self, ast: &FileAst) -> Vec<RefDecl> {
        let prefix = module_prefix(&ast.path);
        let source = ast.source.as_bytes();
        let scope = build_scope(ast.tree.root_node(), source, &prefix);
        let field_types = collect_field_types(ast.tree.root_node(), source);
        let binding_types = collect_binding_types(ast.tree.root_node(), source);
        let mut out = Vec::new();
        collect_refs(
            ast.tree.root_node(),
            source,
            &prefix,
            None,
            &FileContext {
                scope,
                field_types,
                binding_types,
            },
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
            None,
            &self.config,
            &mut out,
        );
        out
    }
}

/// Derives a Rust module path from a file path relative to the crate root, e.g.
/// `src/payment/service.rs` -> `payment::service`, `src/lib.rs` -> `` (crate root).
/// Approximate: doesn't follow `#[path = "..."]` or `mod` declarations that diverge from
/// the filesystem layout — good enough for the structural blast-radius this tool computes,
/// not a claim of full module-resolution correctness.
fn module_prefix(rel_path: &str) -> String {
    let path = rel_path.replace('\\', "/");
    let path = path.strip_prefix("src/").unwrap_or(&path);
    let path = path.strip_suffix(".rs").unwrap_or(path);
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if matches!(segments.last(), Some(&"lib") | Some(&"main") | Some(&"mod")) {
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

/// An `impl` block's type name without its generic parameter list: `Container` for both
/// `impl Container` and `impl<T> Container<T>` (a `generic_type` node whose own `type`
/// field is the plain name) — the qualified path should identify the type, not restate
/// its generics at every impl site.
fn impl_type_name<'a>(impl_item: Node, source: &'a [u8]) -> Option<&'a str> {
    let type_node = impl_item.child_by_field_name("type")?;
    if type_node.kind() == "generic_type" {
        type_node
            .child_by_field_name("type")?
            .utf8_text(source)
            .ok()
    } else {
        type_node.utf8_text(source).ok()
    }
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
    });
}

/// Contract identity strings are bare, project-wide-unique names (`"PaymentCreated"`,
/// `"payments"`, `"POST /payments"`) — never module-prefixed, unlike regular symbols —
/// so a consuming file elsewhere can reference the same contract by writing the same
/// bare name without needing to know which module declared it.
fn push_contract(out: &mut Vec<SymbolDecl>, kind: ContractKind, name: &str, node: Node) {
    out.push(SymbolDecl {
        kind: NodeKind::Contract(kind),
        qualified_path: name.to_string(),
        line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        is_test: false,
        is_generated: false,
    });
}

/// Indexes each of an enum's variants as its own `Field`-kind symbol, qualified
/// `<enum_prefix>::<VariantName>` — so a match arm's `Enum::Variant` pattern (see
/// `emit_variant_refs`) has a real node to resolve against via `Resolver`'s "last two
/// segments" tier, which is what lets `--change "remove variant Enum::Variant"` compute
/// a real blast radius instead of only working at whole-enum granularity.
fn push_enum_variants(
    out: &mut Vec<SymbolDecl>,
    enum_prefix: &str,
    enum_item: Node,
    source: &[u8],
) {
    let mut cursor = enum_item.walk();
    for child in enum_item.children(&mut cursor) {
        if child.kind() != "enum_variant_list" {
            continue;
        }
        let mut variant_cursor = child.walk();
        for variant in child.children(&mut variant_cursor) {
            if variant.kind() != "enum_variant" {
                continue;
            }
            let mut name_cursor = variant.walk();
            let name_node = variant
                .children(&mut name_cursor)
                .find(|n| n.kind() == "identifier");
            if let Some(name_node) = name_node {
                if let Ok(name) = name_node.utf8_text(source) {
                    push(out, NodeKind::Field, enum_prefix, name, variant, false);
                }
            }
        }
    }
}

fn has_test_attribute(function_item: Node, source: &[u8]) -> bool {
    let mut sibling = function_item.prev_sibling();
    while let Some(node) = sibling {
        match node.kind() {
            "attribute_item" => {
                if is_test_attribute(node, source) {
                    return true;
                }
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        sibling = node.prev_sibling();
    }
    false
}

fn is_test_attribute(attribute_item: Node, source: &[u8]) -> bool {
    let mut cursor = attribute_item.walk();
    for child in attribute_item.children(&mut cursor) {
        // The `attribute` node's inner path (`test`, `tokio::test`) isn't exposed as a
        // named field in this grammar — its own text IS the path, with no `#[...]`
        // wrapper or arguments to strip for the bare attributes this checks for.
        if child.kind() == "attribute" {
            if let Ok(path) = child.utf8_text(source) {
                return matches!(path, "test" | "tokio::test" | "async_std::test");
            }
        }
    }
    false
}

/// Walks top-level items and `impl`/inline-`mod` bodies, extracting one `SymbolDecl` per
/// function, struct, enum, and trait — plus, when config says so, an additional bare-
/// named `Contract` declaration for a type that's an event (per `event_strategy`). Does
/// not descend into function/struct/enum bodies — nested items inside a function are out
/// of scope for this phase.
fn walk(
    node: Node,
    source: &[u8],
    prefix: &str,
    config: &DetectorConfig,
    out: &mut Vec<SymbolDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    let is_test = has_test_attribute(child, source);
                    push(out, NodeKind::Function, prefix, name, child, is_test);
                }
            }
            "struct_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if config.event_strategy == EventStrategy::NamingConvention
                        && name.ends_with(config.event_naming_suffix.as_str())
                    {
                        push_contract(out, ContractKind::Event, name, child);
                    }
                }
            }
            "enum_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Type, prefix, name, child, false);
                    if config.event_strategy == EventStrategy::NamingConvention
                        && name.ends_with(config.event_naming_suffix.as_str())
                    {
                        push_contract(out, ContractKind::Event, name, child);
                    }
                    let enum_prefix = join_path(prefix, name);
                    push_enum_variants(out, &enum_prefix, child, source);
                }
            }
            "trait_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    push(out, NodeKind::Trait, prefix, name, child, false);
                }
            }
            "impl_item" => {
                if config.event_strategy == EventStrategy::MarkerTrait {
                    if let Some(trait_node) = child.child_by_field_name("trait") {
                        if let Some(trait_name) = last_identifier_text(trait_node, source) {
                            if trait_name == config.event_marker_trait {
                                if let Some(type_name) = impl_type_name(child, source) {
                                    push_contract(out, ContractKind::Event, type_name, child);
                                }
                            }
                        }
                    }
                }
                if let (Some(type_name), Some(body)) = (
                    impl_type_name(child, source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, type_name);
                    walk(body, source, &new_prefix, config, out);
                }
            }
            "mod_item" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    walk(body, source, &new_prefix, config, out);
                }
            }
            _ => {}
        }
    }
}

/// The rightmost identifier-like leaf in an expression: `validate` for `validate()`,
/// `charge` for `self.charge()` (a `field_expression`), `method` for `Type::method()` (a
/// `scoped_identifier`), `PaymentCreated` for `events::PaymentCreated { .. }` or
/// `&PaymentCreated`. Structural, not type-aware — it doesn't know what `self` or `Type`
/// resolve to, only the name written, which is exactly what the linker needs to match
/// against known symbol/contract names.
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

/// Everything one file says about itself that a call site in it can be resolved against:
/// its imports and own declarations (`FileScope`), plus the declared type of each struct
/// field, which is what makes `self.service.charge()` resolvable rather than a guess.
struct FileContext {
    scope: FileScope,
    field_types: HashMap<String, String>,
    binding_types: HashMap<String, String>,
}

/// Collects this file's `use` declarations and top-level item names into a `FileScope`.
///
/// A name with neither a `use` nor a declaration here is `Opaque`: in Rust it's a macro,
/// a prelude item, or something in another module that this file never named, and none of
/// those are evidence that a same-named symbol elsewhere in the project is the target.
fn build_scope(root: Node, source: &[u8], prefix: &str) -> FileScope {
    let mut scope = FileScope::new(prefix, RefTarget::Opaque);
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                if let Some(argument) = child.child_by_field_name("argument") {
                    collect_use(argument, source, prefix, &[], &mut scope);
                }
            }
            "function_item" | "struct_item" | "enum_item" | "trait_item" | "type_item"
            | "union_item" | "const_item" | "static_item" | "mod_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    scope.declare_local(name);
                }
            }
            _ => {}
        }
    }
    scope
}

/// Walks one `use` tree, accumulating the module path in `path` and recording each leaf
/// (a plain name, an `as` alias, or a `*`) against the module it came from.
///
/// `crate::` and `self::` are dropped: this adapter's module paths are already
/// crate-relative (see `module_prefix`). `super::` climbs one segment out of the current
/// module, which is as far as a purely structural reading can honestly go.
fn collect_use(node: Node, source: &[u8], prefix: &str, path: &[String], scope: &mut FileScope) {
    match node.kind() {
        "scoped_identifier" | "scoped_use_list" | "use_wildcard" => {
            let (qualifier_path, leaf) = match node.kind() {
                "use_wildcard" => (node.child_by_field_name("path"), None),
                _ => (
                    node.child_by_field_name("path"),
                    node.child_by_field_name("name"),
                ),
            };
            let mut path = path.to_vec();
            if let Some(qualifier) = qualifier_path {
                extend_use_path(qualifier, source, prefix, &mut path);
            }
            match node.kind() {
                "use_wildcard" => scope.add_wildcard(path.join("::")),
                "scoped_use_list" => {
                    if let Some(list) = node.child_by_field_name("list") {
                        collect_use(list, source, prefix, &path, scope);
                    }
                }
                _ => {
                    if let Some(name) = leaf.and_then(|n| n.utf8_text(source).ok()) {
                        scope.add_import(name, path.join("::"));
                    }
                }
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !matches!(child.kind(), "," | "{" | "}") {
                    collect_use(child, source, prefix, path, scope);
                }
            }
        }
        "use_as_clause" => {
            // `use a::b::C as D` — D is what call sites here write, a::b is where it lives.
            let mut path = path.to_vec();
            if let Some(original) = node.child_by_field_name("path") {
                if original.kind() == "scoped_identifier" {
                    if let Some(qualifier) = original.child_by_field_name("path") {
                        extend_use_path(qualifier, source, prefix, &mut path);
                    }
                }
            }
            if let Some(alias) = field_text(node, "alias", source) {
                scope.add_import(alias, path.join("::"));
            }
        }
        "identifier" => {
            if let Ok(name) = node.utf8_text(source) {
                scope.add_import(name, path.join("::"));
            }
        }
        _ => {}
    }
}

/// Appends a `use` path's segments to `path`, normalizing the crate-relative prefixes
/// this adapter's `module_prefix` doesn't use.
fn extend_use_path(node: Node, source: &[u8], prefix: &str, path: &mut Vec<String>) {
    match node.kind() {
        "scoped_identifier" => {
            if let Some(inner) = node.child_by_field_name("path") {
                extend_use_path(inner, source, prefix, path);
            }
            if let Some(name) = node.child_by_field_name("name") {
                extend_use_path(name, source, prefix, path);
            }
        }
        "crate" | "self" => {}
        "super" => {
            // One module out from this file's own module.
            if path.is_empty() {
                let mut segments: Vec<&str> =
                    prefix.split("::").filter(|s| !s.is_empty()).collect();
                segments.pop();
                path.extend(segments.into_iter().map(str::to_string));
            }
        }
        _ => {
            if let Ok(text) = node.utf8_text(source) {
                path.push(text.to_string());
            }
        }
    }
}

/// Maps each struct field name in this file to its declared type name, so a call through
/// a field (`self.service.charge()`) can be resolved via the type's own import rather
/// than falling back to the field name alone.
///
/// Keyed by field name across the whole file rather than per struct: the enclosing struct
/// at a `self.field` call site is knowable, but two structs in one file sharing a field
/// name *and* disagreeing on its type is rare enough that carrying the extra state buys
/// nothing. Worst case the wrong type resolves and the call comes back `Probable`.
fn collect_field_types(root: Node, source: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    collect_field_types_inner(root, source, &mut out);
    out
}

fn collect_field_types_inner(node: Node, source: &[u8], out: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            if let (Some(name), Some(type_node)) = (
                field_text(child, "name", source),
                child.child_by_field_name("type"),
            ) {
                if let Some(type_name) = base_type_name(type_node, source) {
                    out.insert(name.to_string(), type_name.to_string());
                }
            }
            continue;
        }
        collect_field_types_inner(child, source, out);
    }
}

/// Maps each `let`-bound name in this file to the type it holds, where the binding says
/// so plainly: an annotation (`let s: PaymentService = ...`), a struct literal, a unit
/// struct, or an associated-function call (`PaymentService::new()`). That covers how a
/// receiver acquires its type in most Rust call sites without doing type inference —
/// anything less obvious is left out, and its method calls stay `Opaque`.
///
/// Keyed by name across the file for the same reason `collect_field_types` is: shadowing
/// the same name with a different type in one file is rare, and getting it wrong costs a
/// confidence tier, not a missed caller.
fn collect_binding_types(root: Node, source: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    collect_binding_types_inner(root, source, &mut out);
    out
}

fn collect_binding_types_inner(node: Node, source: &[u8], out: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "let_declaration" {
            if let Some(name) = child
                .child_by_field_name("pattern")
                .filter(|p| p.kind() == "identifier")
                .and_then(|p| p.utf8_text(source).ok())
            {
                let declared = child
                    .child_by_field_name("type")
                    .and_then(|t| base_type_name(t, source))
                    .or_else(|| {
                        child
                            .child_by_field_name("value")
                            .and_then(|v| expression_type_name(v, source))
                    });
                if let Some(type_name) = declared {
                    out.insert(name.to_string(), type_name.to_string());
                }
            }
        }
        collect_binding_types_inner(child, source, out);
    }
}

/// The type an initializer expression obviously produces, or `None` when saying would
/// mean guessing.
fn expression_type_name<'a>(value: Node, source: &'a [u8]) -> Option<&'a str> {
    match value.kind() {
        // `PaymentService { .. }`
        "struct_expression" => value
            .child_by_field_name("name")
            .and_then(|n| base_type_name(n, source)),
        // `PaymentService::new(...)` — the qualifier names the type.
        "call_expression" => {
            let function = value.child_by_field_name("function")?;
            if function.kind() != "scoped_identifier" {
                return None;
            }
            let mut segments = Vec::new();
            path_segments(function.child_by_field_name("path")?, source, &mut segments);
            let last = segments.last()?;
            starts_uppercase(last).then(|| {
                // Borrowed from the source rather than the owned `segments`, so the
                // returned name outlives this function.
                function
                    .child_by_field_name("path")
                    .and_then(|p| rightmost_identifier(p, source))
            })?
        }
        // A bare unit struct: `let handler = PaymentHandler;`
        "identifier" | "type_identifier" => value
            .utf8_text(source)
            .ok()
            .filter(|text| starts_uppercase(text)),
        _ => None,
    }
}

fn starts_uppercase(text: &str) -> bool {
    text.chars().next().is_some_and(char::is_uppercase)
}

/// The last identifier in a path node, borrowed from the source text.
fn rightmost_identifier<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if node.kind() == "scoped_identifier" {
        return node
            .child_by_field_name("name")
            .and_then(|n| rightmost_identifier(n, source));
    }
    node.utf8_text(source).ok()
}

/// The bare type name inside whatever wrappers a field declaration uses — `PaymentService`
/// for `PaymentService`, `&PaymentService`, `Arc<PaymentService>`, `Option<PaymentService>`.
/// The first `type_identifier` in the subtree is that name for every shape this handles;
/// a generic container's own name is a `type_identifier` too, so `Arc<T>` yields `Arc`
/// only when it has no inner named type, which resolves to nothing and is harmless.
fn base_type_name<'a>(node: Node, source: &'a [u8]) -> Option<&'a str> {
    if node.kind() == "generic_type" {
        if let Some(arguments) = node.child_by_field_name("type_arguments") {
            if let Some(inner) = base_type_name(arguments, source) {
                return Some(inner);
            }
        }
    }
    if node.kind() == "type_identifier" {
        return node.utf8_text(source).ok();
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = base_type_name(child, source) {
            return Some(found);
        }
    }
    None
}

/// Where a call's callee expression points, as far as this file can say.
///
/// The shapes that carry evidence: a bare name the file imports or declares, a path
/// (`crate::repo::save`, `PaymentService::charge`) whose qualifier normalizes to a
/// module, and `self.method()` / `self.field.method()` where the field's declared type is
/// known. Anything else — a method call on a local, a parameter, a chained expression —
/// is `Opaque`, because the receiver's type needs the type inference this adapter
/// deliberately doesn't do.
fn call_target(func: Node, source: &[u8], ctx: &FileContext) -> RefTarget {
    match func.kind() {
        "identifier" => func
            .utf8_text(source)
            .map(|name| ctx.scope.bare(name))
            .unwrap_or(RefTarget::Opaque),
        "scoped_identifier" => match func.child_by_field_name("path") {
            Some(path) => {
                let mut segments = Vec::new();
                path_segments(path, source, &mut segments);
                qualifier_target(&segments, ctx)
            }
            None => RefTarget::Opaque,
        },
        // `foo::<T>()` — the turbofish wraps the real callee one level down.
        "generic_function" => match func.child_by_field_name("function") {
            Some(inner) => call_target(inner, source, ctx),
            None => RefTarget::Opaque,
        },
        "field_expression" => match func.child_by_field_name("value") {
            Some(receiver) => receiver_target(receiver, source, ctx),
            None => RefTarget::Opaque,
        },
        _ => RefTarget::Opaque,
    }
}

/// The target for a method call's receiver: `self` is this file's own module, `self.field`
/// resolves through the field's declared type, and everything else is `Opaque`.
fn receiver_target(receiver: Node, source: &[u8], ctx: &FileContext) -> RefTarget {
    match receiver.kind() {
        "self" => ctx.scope.own(),
        "identifier" => receiver
            .utf8_text(source)
            .map(|name| binding_target(name, ctx))
            .unwrap_or(RefTarget::Opaque),
        "field_expression" => {
            let is_self_field = receiver
                .child_by_field_name("value")
                .is_some_and(|v| v.kind() == "self");
            if !is_self_field {
                return RefTarget::Opaque;
            }
            receiver
                .child_by_field_name("field")
                .and_then(|f| f.utf8_text(source).ok())
                .and_then(|field| ctx.field_types.get(field))
                .map(|type_name| ctx.scope.qualified(type_name))
                .unwrap_or(RefTarget::Opaque)
        }
        _ => RefTarget::Opaque,
    }
}

/// Where a call on a plain identifier receiver points: through the binding's known type
/// when there is one (`let handler = PaymentHandler; handler.run()`), otherwise treating
/// the identifier itself as a qualifier — a module alias or an imported type.
fn binding_target(name: &str, ctx: &FileContext) -> RefTarget {
    match ctx.binding_types.get(name) {
        Some(type_name) => ctx.scope.qualified(type_name),
        None => ctx.scope.qualified(name),
    }
}

/// Flattens a path expression into its segments: `["crate", "repo"]` for `crate::repo`.
fn path_segments(node: Node, source: &[u8], out: &mut Vec<String>) {
    if node.kind() == "scoped_identifier" {
        if let Some(path) = node.child_by_field_name("path") {
            path_segments(path, source, out);
        }
        if let Some(name) = node.child_by_field_name("name") {
            path_segments(name, source, out);
        }
        return;
    }
    if let Ok(text) = node.utf8_text(source) {
        out.push(text.to_string());
    }
}

/// Turns a call's qualifier segments into a module, normalizing the crate-relative
/// prefixes this adapter's `module_prefix` doesn't use (`crate::`, `self::`, `super::`)
/// and letting an imported or locally-declared leading name win over reading the whole
/// qualifier as a path — `PaymentService::charge()` names a type, not a module, and only
/// this file's `use` says where that type lives.
///
/// A qualifier that resolves to no module in this project (`std::mem::swap`, `Vec::new`)
/// deliberately yields a `Module` that matches nothing, so the linker drops the edge
/// rather than falling back to the name alone.
fn qualifier_target(segments: &[String], ctx: &FileContext) -> RefTarget {
    let Some(first) = segments.first() else {
        return RefTarget::Opaque;
    };
    let rest = || segments[1..].join("::");
    match first.as_str() {
        "crate" => RefTarget::Module(rest()),
        "self" => RefTarget::Module(prepend_module(ctx.scope.module(), &rest())),
        "super" => {
            let mut parent: Vec<&str> = ctx
                .scope
                .module()
                .split("::")
                .filter(|s| !s.is_empty())
                .collect();
            parent.pop();
            RefTarget::Module(prepend_module(&parent.join("::"), &rest()))
        }
        _ => match ctx.scope.qualified(first) {
            RefTarget::Opaque => RefTarget::Module(segments.join("::")),
            resolved => resolved,
        },
    }
}

fn prepend_module(base: &str, rest: &str) -> String {
    match (base.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (false, true) => base.to_string(),
        (false, false) => format!("{base}::{rest}"),
    }
}

/// Where a call found in a macro's flat token stream points. There's no parse tree to
/// read here (see `scan_macro_calls`), only the tokens either side of the name: a `::`
/// run in front is a path qualifier, a `.` in front makes it a method call on a receiver
/// with no type to look up, and neither means a bare name.
fn macro_call_target(ident: Node, source: &[u8], ctx: &FileContext) -> RefTarget {
    let mut segments: Vec<String> = Vec::new();
    let mut previous = ident.prev_sibling();
    while let Some(separator) = previous {
        match separator.utf8_text(source) {
            Ok("::") => {}
            // `handler.charge()` inside a macro: the receiver is the token in front of
            // the dot, which a `let` binding may well have given a knowable type.
            Ok(".") => {
                return separator
                    .prev_sibling()
                    .and_then(|receiver| receiver.utf8_text(source).ok())
                    .map(|name| binding_target(name, ctx))
                    .unwrap_or(RefTarget::Opaque);
            }
            _ => break,
        }
        let Some(segment) = separator.prev_sibling() else {
            break;
        };
        let Ok(text) = segment.utf8_text(source) else {
            break;
        };
        segments.push(text.to_string());
        previous = segment.prev_sibling();
    }

    if segments.is_empty() {
        return ident
            .utf8_text(source)
            .map(|name| ctx.scope.bare(name))
            .unwrap_or(RefTarget::Opaque);
    }
    segments.reverse();
    qualifier_target(&segments, ctx)
}

/// Walks the same item shapes as `walk`, but descends into function bodies (which `walk`
/// deliberately doesn't) to find `call_expression`s, recording each as a `RefDecl` from
/// the enclosing function. `current_fn` is `None` outside any function body, so a call
/// expression found there (e.g. in a `const` initializer) is silently unattributed
/// rather than mis-attributed.
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
            "function_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_refs(body, source, prefix, Some(&qualified), ctx, out);
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
                // arguments: `build().charge()` calls `build` too.
                if let Some(callee) = child.child_by_field_name("function") {
                    collect_refs(callee, source, prefix, current_fn, ctx, out);
                }
                if let Some(args) = child.child_by_field_name("arguments") {
                    collect_refs(args, source, prefix, current_fn, ctx, out);
                }
            }
            "impl_item" => {
                if let (Some(type_name), Some(body)) = (
                    impl_type_name(child, source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, type_name);
                    collect_refs(body, source, &new_prefix, current_fn, ctx, out);
                }
            }
            "mod_item" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_refs(body, source, &new_prefix, current_fn, ctx, out);
                }
            }
            "macro_invocation" => {
                // Macro arguments (`assert!(handler.charge())`, `assert_eq!(...)`,
                // `vec![...]`) are NOT parsed into `call_expression`/`field_expression`
                // nodes — tree-sitter can't know what a macro does with its tokens, so a
                // macro's `token_tree` is a flat, unstructured token sequence. Ordinary
                // recursion here would find nothing, silently missing calls, which is
                // wrong given how common `assert!(some_call())` is in tests specifically
                // — the one place this tool most needs to see through. So macro bodies
                // get their own scan instead of the structured `call_expression` walk.
                if let Some(from) = current_fn {
                    let mut mc = child.walk();
                    for grandchild in child.children(&mut mc) {
                        if grandchild.kind() == "token_tree" {
                            scan_macro_calls(grandchild, source, from, ctx, out);
                        }
                    }
                }
            }
            "match_pattern" => {
                // `Enum::Variant` (and `Enum::Variant(..)`) inside a match arm's pattern
                // parses as a `scoped_identifier` — its whole text is exactly the
                // `Enum::Variant` shape `Resolver`'s "last two segments" tier resolves,
                // so this needs no special-casing beyond finding those nodes. A pattern
                // has no calls to find, so this doesn't fall through to generic recursion.
                if let Some(from) = current_fn {
                    emit_variant_refs(child, source, from, out);
                }
            }
            _ => {
                collect_refs(child, source, prefix, current_fn, ctx, out);
            }
        }
    }
}

/// Finds every `scoped_identifier` (`Enum::Variant`) within a match arm's pattern and
/// records it as a `References` edge from the enclosing function — including inside an
/// or-pattern (`Pending | Failed(_)`), which just means more than one match here. Stops
/// descending once it matches one, since a `scoped_identifier`'s own children (the two
/// `identifier`s either side of `::`) aren't further patterns to find.
fn emit_variant_refs(node: Node, source: &[u8], from: &str, out: &mut Vec<RefDecl>) {
    if node.kind() == "scoped_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            out.push(RefDecl {
                from_qualified_path: from.to_string(),
                to_name: text.to_string(),
                kind: EdgeKind::References,
                // An `Enum::Variant` pattern already names its own scope — that *is* the
                // evidence, and it resolves on the linker's last-two-segments tier.
                to_target: RefTarget::Unscoped,
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        emit_variant_refs(child, source, from, out);
    }
}

/// Scans a macro's flat token sequence for `name(` / `name.method(` shapes: any
/// identifier-like token immediately followed (as its next sibling) by a
/// parenthesized `token_tree` is treated as a call to that name. Recurses into nested
/// token trees (nested macro/call arguments) to catch calls at any depth.
fn scan_macro_calls(
    node: Node,
    source: &[u8],
    from: &str,
    ctx: &FileContext,
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "field_identifier") {
            if let Some(sibling) = child.next_sibling() {
                if sibling.kind() == "token_tree"
                    && sibling.child(0).is_some_and(|c| c.kind() == "(")
                {
                    if let Ok(name) = child.utf8_text(source) {
                        let to_target = if child.kind() == "field_identifier" {
                            RefTarget::Opaque
                        } else {
                            macro_call_target(child, source, ctx)
                        };
                        out.push(RefDecl {
                            from_qualified_path: from.to_string(),
                            to_name: name.to_string(),
                            kind: EdgeKind::Calls,
                            to_target,
                        });
                    }
                }
            }
        }
        scan_macro_calls(child, source, from, ctx, out);
    }
}

const HTTP_VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// Strips the surrounding quotes from a Rust string literal's raw source text. Doesn't
/// handle escape sequences — route paths and SQL in practice don't need them.
fn string_literal_text(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "string_literal" {
        return None;
    }
    let text = node.utf8_text(source).ok()?;
    Some(text.trim_matches('"').to_string())
}

fn first_string_literal(node: Node, source: &[u8]) -> Option<String> {
    if let Some(text) = string_literal_text(node, source) {
        return Some(text);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(text) = first_string_literal(child, source) {
            return Some(text);
        }
    }
    None
}

/// A `.route(path, verb(handler))` axum registration call, if `call` is one: the HTTP
/// verb (uppercased) and path from the first two arguments, plus the handler's bare name
/// from the verb call's own argument.
fn axum_route_call(call: Node, source: &[u8]) -> Option<(String, String, String)> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "field_expression" {
        return None;
    }
    let field = field_text(function, "field", source)?;
    if field != "route" {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let path_arg = arguments.named_child(0)?;
    let path = string_literal_text(path_arg, source)?;

    let verb_call = arguments.named_child(1)?;
    if verb_call.kind() != "call_expression" {
        return None;
    }
    let verb_fn = verb_call.child_by_field_name("function")?;
    let verb = verb_fn.utf8_text(source).ok()?.to_lowercase();
    if !HTTP_VERBS.contains(&verb.as_str()) {
        return None;
    }
    let verb_args = verb_call.child_by_field_name("arguments")?;
    let handler_arg = verb_args.named_child(0)?;
    let handler = last_identifier_text(handler_arg, source)?.to_string();

    Some((verb.to_uppercase(), path, handler))
}

fn extract_table_refs(sql: &str) -> Vec<(String, ContractRole)> {
    let tokens: Vec<&str> = sql
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        let role = match tokens[i].to_uppercase().as_str() {
            "FROM" | "JOIN" => Some(ContractRole::Reads),
            "INTO" | "UPDATE" => Some(ContractRole::Writes),
            _ => None,
        };
        if let Some(role) = role {
            if let Some(table) = tokens.get(i + 1) {
                out.push((table.to_string(), role));
            }
        }
    }
    out
}

/// Walks the same item shapes as `collect_refs`, looking for contract relationships:
/// axum route registrations (API), event struct construction and typed parameters
/// (EVENTS), and `sqlx` query macros (DATABASE). Each candidate is emitted regardless of
/// whether it turns out to be real — e.g. every typed function parameter is offered as a
/// possible event consumer — and the linker's exact-match-only contract resolution is
/// what actually filters out the ones that aren't, so this adapter doesn't need to know
/// in advance which type names are events.
fn collect_contracts(
    node: Node,
    source: &[u8],
    prefix: &str,
    current_fn: Option<&str>,
    config: &DetectorConfig,
    out: &mut Vec<ContractRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(name) = field_text(child, "name", source) {
                    let qualified = join_path(prefix, name);
                    if let Some(params) = child.child_by_field_name("parameters") {
                        collect_param_events(params, source, &qualified, out);
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_contracts(body, source, prefix, Some(&qualified), config, out);
                    }
                }
            }
            "call_expression" => {
                if config.api_frameworks.iter().any(|f| f == "axum") {
                    if let Some((verb, path, handler)) = axum_route_call(child, source) {
                        out.push(ContractRef {
                            contract_kind: ContractKind::ApiRoute,
                            contract_id: format!("{verb} {path}"),
                            symbol_name: handler,
                            role: ContractRole::Produces,
                        });
                    }
                }
                if let Some(args) = child.child_by_field_name("arguments") {
                    collect_contracts(args, source, prefix, current_fn, config, out);
                }
            }
            "struct_expression" => {
                if let Some(from) = current_fn {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Some(name) = last_identifier_text(name_node, source) {
                            out.push(ContractRef {
                                contract_kind: ContractKind::Event,
                                contract_id: name.to_string(),
                                symbol_name: from.to_string(),
                                role: ContractRole::Produces,
                            });
                        }
                    }
                }
            }
            "macro_invocation" => {
                if let (Some(from), Some(macro_path)) =
                    (current_fn, field_text(child, "macro", source))
                {
                    let macro_name = macro_path.rsplit("::").next().unwrap_or(macro_path);
                    if config.database_macros.iter().any(|m| m == macro_name) {
                        if let Some(sql) = first_string_literal(child, source) {
                            for (table, role) in extract_table_refs(&sql) {
                                out.push(ContractRef {
                                    contract_kind: ContractKind::Table,
                                    contract_id: table,
                                    symbol_name: from.to_string(),
                                    role,
                                });
                            }
                        }
                    }
                }
            }
            "impl_item" => {
                if let (Some(type_name), Some(body)) = (
                    impl_type_name(child, source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, type_name);
                    collect_contracts(body, source, &new_prefix, current_fn, config, out);
                }
            }
            "mod_item" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_contracts(body, source, &new_prefix, current_fn, config, out);
                }
            }
            _ => {
                collect_contracts(child, source, prefix, current_fn, config, out);
            }
        }
    }
}

/// Offers every typed parameter of a function as a possible event consumer — see
/// `collect_contracts`' doc comment for why over-offering is safe.
fn collect_param_events(
    parameters: Node,
    source: &[u8],
    qualified_fn: &str,
    out: &mut Vec<ContractRef>,
) {
    let mut cursor = parameters.walk();
    for param in parameters.named_children(&mut cursor) {
        if param.kind() != "parameter" {
            continue;
        }
        let Some(type_node) = param.child_by_field_name("type") else {
            continue;
        };
        let Some(name) = last_identifier_text(type_node, source) else {
            continue;
        };
        out.push(ContractRef {
            contract_kind: ContractKind::Event,
            contract_id: name.to_string(),
            symbol_name: qualified_fn.to_string(),
            role: ContractRole::Consumes,
        });
    }
}
