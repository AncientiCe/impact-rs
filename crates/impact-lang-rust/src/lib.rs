use std::path::Path;

use impact_core::{
    ContractKind, ContractRef, ContractRole, DetectorConfig, EdgeKind, EventStrategy, FileAst,
    LanguageAdapter, NodeKind, RefDecl, SymbolDecl,
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
    out: &mut Vec<RefDecl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
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
            "impl_item" => {
                if let (Some(type_name), Some(body)) = (
                    impl_type_name(child, source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, type_name);
                    collect_refs(body, source, &new_prefix, current_fn, out);
                }
            }
            "mod_item" => {
                if let (Some(name), Some(body)) = (
                    field_text(child, "name", source),
                    child.child_by_field_name("body"),
                ) {
                    let new_prefix = join_path(prefix, name);
                    collect_refs(body, source, &new_prefix, current_fn, out);
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
                            scan_macro_calls(grandchild, source, from, out);
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
                collect_refs(child, source, prefix, current_fn, out);
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
fn scan_macro_calls(node: Node, source: &[u8], from: &str, out: &mut Vec<RefDecl>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "field_identifier") {
            if let Some(sibling) = child.next_sibling() {
                if sibling.kind() == "token_tree"
                    && sibling.child(0).is_some_and(|c| c.kind() == "(")
                {
                    if let Ok(name) = child.utf8_text(source) {
                        out.push(RefDecl {
                            from_qualified_path: from.to_string(),
                            to_name: name.to_string(),
                            kind: EdgeKind::Calls,
                        });
                    }
                }
            }
        }
        scan_macro_calls(child, source, from, out);
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
