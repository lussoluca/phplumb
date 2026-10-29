//! JetBrains `phpstorm-stubs`: the declarations of PHP's built-in classes, functions
//! and constants, read as PHP `PHP_VERSION` has them (see `data/README.md`).

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::index::Index;
use crate::metadata::{MetaValue, Metadata};
use crate::phpdoc::{self, NameContext, TypeExpr};
use crate::summary::{Declaration, FileSummary, SymbolKind, pick_type, summarize};

/// The PHP version whose declarations the stubs give.
pub const PHP_VERSION: (u32, u32) = (8, 5);

/// Marks a declaration available only from and to some PHP versions.
const AVAILABLE: &str = "JetBrains\\PhpStorm\\Internal\\PhpStormStubsElementAvailable";

/// Gives a declaration's type per PHP version, with a default.
const TYPE_AWARE: &str = "JetBrains\\PhpStorm\\Internal\\LanguageLevelTypeAware";

/// The directory of the stub files: `$PHPLUMB_STUBS`, `stubs` beside the engine
/// binary, or the copy in the source tree the engine was built from, whichever
/// exists first.
pub fn default_dir() -> Option<PathBuf> {
    let configured = std::env::var_os("PHPLUMB_STUBS").map(PathBuf::from);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("stubs")));
    let built = Some(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/stubs"
    )));
    [configured, beside, built]
        .into_iter()
        .flatten()
        .find(|dir| dir.is_dir())
}

/// The index of the stubs in `dir`, kept for the process once it is built; none when
/// the directory cannot be read or holds no PHP file, which the next call tries
/// again.
pub(crate) fn load(dir: &Path) -> Option<Arc<Index>> {
    static LOADED: OnceLock<Mutex<HashMap<PathBuf, Arc<Index>>>> = OnceLock::new();
    let dir = dir.canonicalize().ok()?;
    let mut loaded = LOADED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(index) = loaded.get(&dir) {
        return Some(Arc::clone(index));
    }
    let index = Index::build_stubs(&dir)
        .ok()
        .filter(|index| index.declarations().next().is_some())
        .map(Arc::new)?;
    loaded.insert(dir, Arc::clone(&index));
    Some(index)
}

/// The summary of a stub file as PHP `PHP_VERSION` has it: declarations it lacks
/// are dropped, each function and member is kept once (its last declaration), and
/// return types follow `LanguageLevelTypeAware`.
pub fn summarize_stub(source: &[u8]) -> FileSummary {
    let mut summary = summarize(source);
    summary.declarations.retain(|declaration| {
        let metadata = match &declaration.class {
            Some(shape) => &shape.metadata,
            None => &declaration.metadata,
        };
        available(metadata)
    });
    for declaration in &mut summary.declarations {
        apply_versions(declaration);
    }
    keep_last(&mut summary.declarations, |declaration| {
        (declaration.kind == SymbolKind::Function).then(|| declaration.fqn.to_ascii_lowercase())
    });
    summary
}

fn apply_versions(declaration: &mut Declaration) {
    let names = StubNames {
        namespace: namespace_of(&declaration.fqn),
    };
    if let Some(ty) = language_level_type(&declaration.metadata, &names) {
        declaration.returns = pick_type(doc_returns(declaration.doc.as_deref(), &names), Some(ty));
    }
    let Some(shape) = &mut declaration.class else {
        return;
    };
    shape.members.retain(|member| available(&member.metadata));
    for member in &mut shape.members {
        if let Some(ty) = language_level_type(&member.metadata, &names) {
            member.ty = pick_type(doc_returns(member.doc.as_deref(), &names), Some(ty));
        }
    }
    keep_last(&mut shape.members, |member| {
        Some((member.kind, member.name.to_ascii_lowercase()))
    });
}

/// Whether `metadata` leaves the declaration available in `PHP_VERSION`.
fn available(metadata: &[Metadata]) -> bool {
    metadata
        .iter()
        .filter(|meta| meta.name.eq_ignore_ascii_case(AVAILABLE))
        .all(|meta| {
            let from = argument(meta, "from", 0).and_then(version);
            let to = argument(meta, "to", 1).and_then(version);
            from.is_none_or(|from| from <= PHP_VERSION) && to.is_none_or(|to| PHP_VERSION <= to)
        })
}

/// The type `LanguageLevelTypeAware` in `metadata` gives in `PHP_VERSION`: the entry
/// of the newest version not above it, else the default; none when that is empty.
fn language_level_type(metadata: &[Metadata], names: &StubNames) -> Option<TypeExpr> {
    let meta = metadata
        .iter()
        .find(|meta| meta.name.eq_ignore_ascii_case(TYPE_AWARE))?;
    let by_version = match meta.arguments.iter().find(|(name, _)| name.is_none()) {
        Some((_, MetaValue::List(entries))) => entries
            .iter()
            .filter_map(|(key, value)| match value {
                MetaValue::String(text) => Some((version(key.as_deref()?)?, text.as_str())),
                _ => None,
            })
            .filter(|(at, _)| *at <= PHP_VERSION)
            .max_by_key(|(at, _)| *at)
            .map(|(_, text)| text),
        _ => None,
    };
    let text = by_version.or_else(|| argument_text(meta, "default"))?;
    (!text.trim().is_empty()).then(|| phpdoc::parse_type(text, names))
}

/// The `@return` type of `doc`.
fn doc_returns(doc: Option<&str>, names: &StubNames) -> Option<TypeExpr> {
    phpdoc::parse_docblock(doc?, names).returns
}

/// The string argument named `name`, or the positional one at `position`.
fn argument<'a>(meta: &'a Metadata, name: &str, position: usize) -> Option<&'a str> {
    argument_text(meta, name).or_else(|| {
        match meta
            .arguments
            .iter()
            .filter(|(name, _)| name.is_none())
            .nth(position)
        {
            Some((_, MetaValue::String(text))) => Some(text.as_str()),
            _ => None,
        }
    })
}

fn argument_text<'a>(meta: &'a Metadata, name: &str) -> Option<&'a str> {
    meta.arguments.iter().find_map(|(key, value)| match value {
        MetaValue::String(text) if key.as_deref() == Some(name) => Some(text.as_str()),
        _ => None,
    })
}

/// `(major, minor)` from the first two dot-separated numbers of `text`.
fn version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |minor| minor.parse().ok())?;
    Some((major, minor))
}

/// Keeps, for each key `key_of` gives, only the last item with that key; items
/// without a key stay. Order is kept.
fn keep_last<T, K: Eq + Hash>(items: &mut Vec<T>, key_of: impl Fn(&T) -> Option<K>) {
    let mut seen: HashSet<K> = HashSet::new();
    let mut keep: Vec<bool> = items
        .iter()
        .rev()
        .map(|item| key_of(item).is_none_or(|key| seen.insert(key)))
        .collect();
    keep.reverse();
    let mut flags = keep.into_iter();
    items.retain(|_| flags.next().unwrap_or(true));
}

fn namespace_of(fqn: &str) -> String {
    fqn.rsplit_once('\\')
        .map(|(namespace, _)| namespace.to_string())
        .unwrap_or_default()
}

/// Class names in stub type strings: relative to the declaration's namespace.
struct StubNames {
    namespace: String,
}

impl NameContext for StubNames {
    fn resolve_class(&self, name: &str) -> String {
        match name.strip_prefix('\\') {
            Some(qualified) => qualified.to_string(),
            None if self.namespace.is_empty() => name.to_string(),
            None => format!("{}\\{name}", self.namespace),
        }
    }
}
