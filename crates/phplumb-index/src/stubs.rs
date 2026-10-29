//! JetBrains `phpstorm-stubs`: the declarations of PHP's built-in classes, functions
//! and constants, read as PHP `PHP_VERSION` has them (see `data/README.md`).

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::index::Index;
use crate::metadata::{MetaValue, Metadata};
use crate::phpdoc::{self, NameContext, TypeExpr};
use crate::summary::{Declaration, FileSummary, MemberKind, SymbolKind, pick_type, summarize};

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
/// return types and signatures follow `LanguageLevelTypeAware`.
pub fn summarize_stub(source: &[u8]) -> FileSummary {
    let mut summary = summarize(source);
    summary.declarations.retain(|declaration| {
        let metadata = match &declaration.class {
            Some(shape) => &shape.metadata,
            None => &declaration.metadata,
        };
        available(metadata)
    });
    let imports = imports(source);
    for declaration in &mut summary.declarations {
        apply_versions(declaration, &imports);
    }
    keep_last(&mut summary.declarations, |declaration| {
        (declaration.kind == SymbolKind::Function).then(|| declaration.fqn.to_ascii_lowercase())
    });
    summary
}

fn apply_versions(declaration: &mut Declaration, imports: &str) {
    let names = StubNames {
        namespace: namespace_of(&declaration.fqn),
    };
    if let Some(ty) = language_level_type(&declaration.metadata, &names) {
        declaration.returns = pick_type(doc_returns(declaration.doc.as_deref(), &names), Some(ty));
    }
    if declaration.kind == SymbolKind::Function {
        declaration.signature = plain_signature(
            &declaration.signature,
            language_level_text(&declaration.metadata).as_deref(),
            imports,
        );
    }
    let Some(shape) = &mut declaration.class else {
        return;
    };
    shape.members.retain(|member| available(&member.metadata));
    for member in &mut shape.members {
        if let Some(ty) = language_level_type(&member.metadata, &names) {
            member.ty = pick_type(doc_returns(member.doc.as_deref(), &names), Some(ty));
        }
        if member.kind == MemberKind::Method {
            member.signature = plain_signature(
                &member.signature,
                language_level_text(&member.metadata).as_deref(),
                imports,
            );
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

/// The type `LanguageLevelTypeAware` in `metadata` gives in `PHP_VERSION`.
fn language_level_type(metadata: &[Metadata], names: &StubNames) -> Option<TypeExpr> {
    language_level_text(metadata).map(|text| phpdoc::parse_type(&text, names))
}

/// The type text `LanguageLevelTypeAware` in `metadata` gives in `PHP_VERSION`: the
/// entry of the newest version not above it, else the default; none when that is
/// empty.
fn language_level_text(metadata: &[Metadata]) -> Option<String> {
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
    let text = by_version
        .or_else(|| argument_text(meta, "default"))?
        .trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A function or method signature as PHP `PHP_VERSION` declares it: parameters it
/// lacks are dropped, parameter attributes are dropped, `LanguageLevelTypeAware`
/// leaves its type before an untyped parameter, and `returns` is added when the
/// signature declares no return type. `imports` are the stub file's `use` lines,
/// which name the attributes.
fn plain_signature(signature: &str, returns: Option<&str>, imports: &str) -> String {
    let mut out = signature.to_string();
    if signature.contains("#[")
        && let Some(open) = signature.find('(')
        && let Some(close) = closing(signature, open)
    {
        let parameters: Vec<String> = split_top_level(&signature[open + 1..close])
            .into_iter()
            .filter_map(|parameter| plain_parameter(parameter, imports))
            .collect();
        out = format!(
            "{}({}){}",
            &signature[..open],
            parameters.join(", "),
            &signature[close + 1..]
        );
    }
    match returns {
        Some(returns) if out.trim_end().ends_with(')') => {
            format!("{}: {returns}", out.trim_end())
        }
        _ => out,
    }
}

/// A parameter without its attributes, typed as `LanguageLevelTypeAware` says;
/// none when PHP `PHP_VERSION` lacks it.
fn plain_parameter(parameter: &str, imports: &str) -> Option<String> {
    let mut rest = parameter.trim();
    let mut groups = String::new();
    while rest.starts_with("#[") {
        let Some(len) = attribute_group_len(rest) else {
            break;
        };
        groups.push_str(&rest[..len]);
        groups.push('\n');
        rest = rest[len..].trim_start();
    }
    if groups.is_empty() {
        return Some(rest.to_string());
    }
    let metadata = attribute_metadata(&groups, imports);
    if !available(&metadata) {
        return None;
    }
    match language_level_text(&metadata) {
        Some(ty) if rest.starts_with(['$', '&', '.']) => Some(format!("{ty} {rest}")),
        _ => Some(rest.to_string()),
    }
}

/// The metadata of attribute `groups`, read by summarizing them on a function of
/// their own after the stub file's `imports`.
fn attribute_metadata(groups: &str, imports: &str) -> Vec<Metadata> {
    let source = format!("<?php\n{imports}{groups}function phplumb_attribute() {{}}\n");
    summarize(source.as_bytes())
        .declarations
        .into_iter()
        .find(|d| d.fqn == "phplumb_attribute")
        .map(|d| d.metadata)
        .unwrap_or_default()
}

/// The top-level `use` lines of a stub file, which import the attribute classes.
fn imports(source: &[u8]) -> String {
    String::from_utf8_lossy(source)
        .lines()
        .filter(|line| line.starts_with("use ") && line.trim_end().ends_with(';'))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// The offset of the `)` closing the `(` at `open`, outside strings.
fn closing(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut found = None;
    scan(&text[open..], |i, byte| {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    found = Some(open + i);
                    return false;
                }
            }
            _ => {}
        }
        true
    });
    found
}

/// `text` cut at its top-level commas, outside strings, brackets and parentheses.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    scan(text, |i, byte| {
        match byte {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        true
    });
    parts.push(&text[start..]);
    parts.retain(|part| !part.trim().is_empty());
    parts
}

/// The length of the attribute group `#[...]` that `text` starts with, quotes and
/// nested brackets included; none when it is not closed.
fn attribute_group_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut found = None;
    scan(&text[1..], |i, byte| {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    found = Some(i + 2);
                    return false;
                }
            }
            _ => {}
        }
        true
    });
    found
}

/// Calls `each` with the offset and byte of every byte of `text` outside quoted
/// strings, until it returns false.
fn scan(text: &str, mut each: impl FnMut(usize, u8) -> bool) {
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (i, byte) in text.bytes().enumerate() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == open {
                quote = None;
            }
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            continue;
        }
        if !each(i, byte) {
            return;
        }
    }
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
