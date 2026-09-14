use std::collections::{HashMap, HashSet};

use crate::adapter::RefTarget;

/// What one source file's own text says about where the names it uses come from: which
/// module each import binds a name to, which names the file declares itself, and what to
/// assume about a name that's neither.
///
/// Every adapter builds one of these from its own syntax (`use` in Rust, `import` in
/// TypeScript and Python, the package clause in Go and Kotlin) and then asks it about
/// each call site. The bookkeeping is identical across languages even though the syntax
/// isn't, so it lives here rather than being written six times.
///
/// The point of all of it is to keep the linker from claiming `Exact` on the strength of
/// a bare name. A file that never imports `prune` is not a caller of some other module's
/// `prune`, however unique that name happens to be project-wide.
#[derive(Debug, Clone)]
pub struct FileScope {
    module: String,
    imports: HashMap<String, String>,
    default_imports: HashMap<String, String>,
    locals: HashSet<String>,
    wildcards: Vec<String>,
    unimported: RefTarget,
}

impl FileScope {
    /// `module` is this file's own module path (the same prefix its symbols are indexed
    /// under). `unimported` is what a name that's neither imported nor declared here
    /// resolves to, which is a real per-language difference: in Rust or TypeScript such a
    /// name is a macro, a global, or something this adapter can't see, so it's `Opaque`;
    /// in Go or Kotlin it's most likely a sibling file in the same package, so an adapter
    /// there passes `Module(<package>)`.
    pub fn new(module: impl Into<String>, unimported: RefTarget) -> Self {
        Self {
            module: module.into(),
            imports: HashMap::new(),
            default_imports: HashMap::new(),
            locals: HashSet::new(),
            wildcards: Vec::new(),
            unimported,
        }
    }

    /// Binds `local_name` (as written at call sites in this file) to the module it was
    /// imported from.
    pub fn add_import(&mut self, local_name: impl Into<String>, module: impl Into<String>) {
        self.imports.insert(local_name.into(), module.into());
    }

    /// Binds `local_name` to the module a *default* import brought it in from — same
    /// bookkeeping as `add_import` (so `qualified()` still treats it as an ordinary
    /// module-scoped name for `Default.staticThing()`-style access), plus a record that
    /// `bare()` prefers: a default import's local name is chosen by the importer, not by
    /// whatever the target module actually calls it, so a bare reference to it should
    /// resolve against the module's default export (`RefTarget::ModuleDefault`) rather
    /// than requiring a same-named symbol to exist there.
    pub fn add_default_import(&mut self, local_name: impl Into<String>, module: impl Into<String>) {
        let local_name = local_name.into();
        let module = module.into();
        self.default_imports
            .insert(local_name.clone(), module.clone());
        self.imports.insert(local_name, module);
    }

    /// A glob import (`use foo::*`, `from foo import *`) — brings in names this file
    /// never states, so it can only be applied when there's exactly one of them.
    pub fn add_wildcard(&mut self, module: impl Into<String>) {
        self.wildcards.push(module.into());
    }

    /// A name this file declares itself (a function, class, type, or trait).
    pub fn declare_local(&mut self, name: impl Into<String>) {
        self.locals.insert(name.into());
    }

    pub fn module(&self) -> &str {
        &self.module
    }

    /// Where a bare `name()` call points.
    pub fn bare(&self, name: &str) -> RefTarget {
        if let Some(module) = self.default_imports.get(name) {
            return RefTarget::ModuleDefault(module.clone());
        }
        if let Some(module) = self.imports.get(name) {
            return RefTarget::Module(module.clone());
        }
        if self.locals.contains(name) {
            return RefTarget::Module(self.module.clone());
        }
        // With two or more glob imports in scope the name could have come from either,
        // and picking one would be a coin flip dressed up as evidence.
        if let [only] = self.wildcards.as_slice() {
            return RefTarget::Module(only.clone());
        }
        self.unimported.clone()
    }

    /// Where a `qualifier::name()` / `qualifier.name()` call points, given the leading
    /// name of the qualifier — a type (`PaymentService::new`), an imported namespace
    /// (`utils.camelize`), or a package (`svc.Handle`). Unlike `bare`, an unrecognized
    /// qualifier is always `Opaque`: it names *something*, and that something not being
    /// in scope here means this file is not the place to guess from.
    pub fn qualified(&self, qualifier: &str) -> RefTarget {
        if let Some(module) = self.imports.get(qualifier) {
            return RefTarget::Module(module.clone());
        }
        if self.locals.contains(qualifier) {
            return RefTarget::Module(self.module.clone());
        }
        RefTarget::Opaque
    }

    /// Where a call on this file's own scope points — `self.method()`, `this.method()`.
    /// The enclosing type is declared here, so its methods are indexed under this file's
    /// module.
    pub fn own(&self) -> RefTarget {
        RefTarget::Module(self.module.clone())
    }
}
