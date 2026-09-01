/// A deterministic parse of a `--change` description into a specific kind of edit. This
/// is a small fixed grammar, not NLP — free text would make identical input produce
/// different traversal decisions across runs, which is exactly the trust this tool sells
/// against "just ask an LLM to read the code." Unparseable input is a hard error with a
/// usage hint, never a best-effort guess.
///
/// Grammar (each line one accepted form):
/// ```text
/// rename <path>
/// rename <path> to <path>
/// remove <path>
/// remove variant <path>::<ident>
/// remove field <path>.<ident>
/// change signature of <path>
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeSpec {
    Rename {
        path: String,
    },
    RemoveSymbol {
        path: String,
    },
    SignatureChange {
        path: String,
    },
    RemoveVariant {
        enum_path: String,
        variant: String,
    },
    /// Coarser than the other variants: this project doesn't track field-access
    /// expressions (`expr.field`) at all — doing so precisely would need to know
    /// `expr`'s static type, which requires the type-checking fidelity this tool
    /// deliberately doesn't have (see the resolution tradeoff in `Resolver`'s docs). So a
    /// field removal's blast radius is approximated as "the containing type's blast
    /// radius" — broader than strictly correct, but honest about what's actually known,
    /// rather than silently pretending field-level precision it doesn't have.
    RemoveField {
        type_path: String,
        field: String,
    },
}

impl ChangeSpec {
    /// The path this change resolves against when computing blast radius. `RemoveVariant`
    /// combines its two parts into the `Enum::Variant` shape `Resolver` already
    /// understands (its "last two segments" tier); `RemoveField` resolves to its
    /// containing type, per the doc on that variant.
    pub fn target_path(&self) -> String {
        match self {
            ChangeSpec::Rename { path }
            | ChangeSpec::RemoveSymbol { path }
            | ChangeSpec::SignatureChange { path } => path.clone(),
            ChangeSpec::RemoveVariant { enum_path, variant } => {
                let enum_short = enum_path.rsplit("::").next().unwrap_or(enum_path);
                format!("{enum_short}::{variant}")
            }
            ChangeSpec::RemoveField { type_path, .. } => type_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "could not parse change description {input:?} — expected one of: \
     \"rename <path>\", \"rename <path> to <path>\", \"remove <path>\", \
     \"remove variant <path>::<ident>\", \"remove field <path>.<ident>\", \
     \"change signature of <path>\""
)]
pub struct ParseChangeError {
    pub input: String,
}

pub fn parse_change(input: &str) -> Result<ChangeSpec, ParseChangeError> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let err = || ParseChangeError {
        input: input.to_string(),
    };
    match tokens.as_slice() {
        ["rename", path] => Ok(ChangeSpec::Rename {
            path: (*path).to_string(),
        }),
        ["rename", path, "to", _new_name] => Ok(ChangeSpec::Rename {
            path: (*path).to_string(),
        }),
        ["remove", "variant", full_path] => full_path
            .rsplit_once("::")
            .map(|(enum_path, variant)| ChangeSpec::RemoveVariant {
                enum_path: enum_path.to_string(),
                variant: variant.to_string(),
            })
            .ok_or_else(err),
        ["remove", "field", full_path] => full_path
            .rsplit_once('.')
            .map(|(type_path, field)| ChangeSpec::RemoveField {
                type_path: type_path.to_string(),
                field: field.to_string(),
            })
            .ok_or_else(err),
        ["remove", path] => Ok(ChangeSpec::RemoveSymbol {
            path: (*path).to_string(),
        }),
        ["change", "signature", "of", path] => Ok(ChangeSpec::SignatureChange {
            path: (*path).to_string(),
        }),
        _ => Err(err()),
    }
}
