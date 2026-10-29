//! Broken references: class, function, method and constant names that resolve to
//! nothing. A reference is reported only when every rule proves its target missing;
//! an unknown receiver, an unknown ancestor, magic or a guard keeps it silent.

use std::collections::HashSet;
use std::path::Path;

use crate::builtins;
use crate::index::Index;
use crate::infer::{self, Receiver};
use crate::summary::{ClassSel, FileSummary, MemberKind, NameKind, NameRef, SymbolKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A broken reference: the range of its name and what is missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub start: u32,
    pub end: u32,
    pub severity: Severity,
    pub message: String,
}

/// Methods every enum has without declaring them.
const ENUM_METHODS: [&str; 3] = ["cases", "from", "tryfrom"];

/// The interfaces PHP gives every enum, whose members PHP fixes.
const ENUM_INTERFACES: [&str; 2] = ["UnitEnum", "BackedEnum"];

pub(crate) fn find(
    index: &Index,
    path: &Path,
    source: &[u8],
    summary: &FileSummary,
) -> Vec<Diagnostic> {
    // `*.api.php` files document hooks with example code that never runs.
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".api.php"))
    {
        return Vec::new();
    }
    let guarded: HashSet<&str> = summary.guarded.iter().map(String::as_str).collect();
    let is_guarded = |name: &str| guarded.contains(name.to_ascii_lowercase().as_str());

    let mut found = Vec::new();
    for name in &summary.names {
        if within(&summary.unchecked, name.start) {
            continue;
        }
        if let Some(diagnostic) = broken_name(index, summary, source, name, &is_guarded) {
            found.push(diagnostic);
        }
    }

    for reference in &summary.static_refs {
        if reference.kind == MemberKind::Property
            || (reference.kind == MemberKind::Constant
                && reference.name.eq_ignore_ascii_case("class"))
            || is_guarded(&reference.name)
        {
            continue;
        }
        let enclosing = summary.enclosing_class(reference.start);
        let enclosing_fqn = enclosing.and_then(|range| range.fqn.clone());
        let (class, dispatch) = match &reference.class {
            ClassSel::Named(fqn) => (Some(fqn.clone()), false),
            ClassSel::SelfKw => (enclosing_fqn.clone(), false),
            ClassSel::StaticKw => (enclosing_fqn.clone(), true),
            ClassSel::ParentKw => (enclosing.and_then(|range| range.parent.clone()), false),
        };
        let in_trait = enclosing_fqn
            .as_deref()
            .is_some_and(|fqn| is_trait(index, fqn));
        let keyword = !matches!(reference.class, ClassSel::Named(_));
        // A closure's scope can be rebound to another class.
        if keyword && within(&summary.closures, reference.start) {
            continue;
        }
        let Some(class) = class else {
            continue;
        };
        if (keyword && in_trait)
            || !missing_member(index, &class, reference.kind, &reference.name, dispatch)
        {
            continue;
        }
        found.push(member_diagnostic(
            reference.start,
            reference.end,
            &class,
            reference.kind,
            &reference.name,
        ));
    }

    for (start, end, name, receiver, kind) in infer::member_accesses(index, path, summary, source) {
        if kind != MemberKind::Method || is_guarded(&name) {
            continue;
        }
        let in_trait = summary
            .enclosing_class(start)
            .and_then(|range| range.fqn.as_deref())
            .is_some_and(|fqn| is_trait(index, fqn));
        let Receiver::Classes(classes) = receiver else {
            continue;
        };
        if in_trait || classes.is_empty() {
            continue;
        }
        // Only `$this` has a type the code itself guarantees. Any other receiver's
        // type may come from a wrong docblock or hide a mock, so its method must also
        // be declared nowhere, by no indexed or built-in class.
        let declared_somewhere = index.has_method_named(&name) || builtins::is_method(&name);
        let certain = on_this(source, start) && !within(&summary.closures, start);
        if !certain && declared_somewhere {
            continue;
        }
        if classes
            .iter()
            .all(|class| missing_member(index, class, kind, &name, true))
        {
            found.push(member_diagnostic(start, end, &classes[0], kind, &name));
        }
    }

    found.sort_by_key(|d| (d.start, d.end));
    found.dedup_by_key(|d| d.start);
    found
}

/// Whether a name of one kind exists, indexed or built in.
type Exists<'a> = &'a dyn Fn(&str) -> bool;

/// The diagnostic for a class, function or constant name that resolves to nothing.
fn broken_name(
    index: &Index,
    summary: &FileSummary,
    source: &[u8],
    name: &NameRef,
    is_guarded: &dyn Fn(&str) -> bool,
) -> Option<Diagnostic> {
    let (exists, severity, what): (Exists, Severity, &str) = match name.kind {
        NameKind::ClassLike => {
            if fetches_class_name(source, name.end) {
                return None;
            }
            let warning =
                within(&summary.hints, name.start) || after_word(source, name.start, b"instanceof");
            let severity = if warning {
                Severity::Warning
            } else {
                Severity::Error
            };
            (
                &|fqn| index.has_class(fqn) || builtins::is_class(fqn),
                severity,
                "class",
            )
        }
        NameKind::Function => (
            &|fqn| index.has_function(fqn) || builtins::is_function(fqn),
            Severity::Error,
            "function",
        ),
        NameKind::Constant => {
            if ["true", "false", "null"]
                .iter()
                .any(|k| name.fqn.eq_ignore_ascii_case(k))
            {
                return None;
            }
            (
                &|fqn| index.has_constant(fqn) || builtins::is_constant(fqn),
                Severity::Error,
                "constant",
            )
        }
    };
    // Unqualified functions and constants in a namespace fall back to the global name.
    let mut candidates = vec![name.fqn.as_str()];
    if name.kind != NameKind::ClassLike
        && !name.qualified
        && !name.imported
        && let Some((_, short)) = name.fqn.rsplit_once('\\')
    {
        candidates.push(short);
    }
    if candidates.iter().any(|fqn| exists(fqn) || is_guarded(fqn)) {
        return None;
    }
    // With paths excluded from the index, a name in a namespace the index knows
    // nothing about may be declared in them.
    let hidden = |fqn: &&str| {
        let namespace = fqn.rsplit_once('\\').map_or("", |(ns, _)| ns);
        !index.knows_namespace(namespace)
    };
    if index.has_exclusions() && candidates.iter().any(hidden) {
        return None;
    }
    Some(Diagnostic {
        start: name.start,
        end: name.end,
        severity,
        message: format!("Undefined {what} '{}'", name.fqn),
    })
}

fn member_diagnostic(
    start: u32,
    end: u32,
    class: &str,
    kind: MemberKind,
    name: &str,
) -> Diagnostic {
    let message = match kind {
        MemberKind::Method => format!("Undefined method '{class}::{name}()'"),
        _ => format!("Undefined class constant '{class}::{name}'"),
    };
    Diagnostic {
        start,
        end,
        severity: Severity::Error,
        message,
    }
}

/// Whether member `name` provably exists on no class a reference through `class`
/// can reach: `class` itself and, when the call dispatches, its subtypes. Silent
/// (false) when `class` is unknown or a trait, or when any of those classes has an
/// ancestor missing from the index or, for methods, magic.
fn missing_member(
    index: &Index,
    class: &str,
    kind: MemberKind,
    name: &str,
    dispatch: bool,
) -> bool {
    let Some(declaration) = index.classes_named(class).into_iter().next() else {
        return false;
    };
    if declaration.declaration.kind == SymbolKind::Trait {
        return false;
    }
    let mut reachable = vec![class.to_string()];
    if dispatch {
        reachable.extend(
            index
                .all_subtypes(class)
                .into_iter()
                .map(|s| s.declaration.fqn.clone()),
        );
    }
    let enum_method =
        kind == MemberKind::Method && ENUM_METHODS.iter().any(|m| name.eq_ignore_ascii_case(m));
    for candidate in &reachable {
        if !index.find_member(candidate, kind, name).is_empty()
            || (kind == MemberKind::Method && aliased(index, candidate, name))
        {
            return false;
        }
        let is_enum = index
            .classes_named(candidate)
            .iter()
            .any(|c| c.declaration.kind == SymbolKind::Enum);
        if enum_method && is_enum {
            return false;
        }
    }
    reachable
        .iter()
        .all(|candidate| fully_known(index, candidate, kind == MemberKind::Method))
}

/// Whether `class` and every ancestor (parents, interfaces, traits) are indexed and,
/// when `check_magic`, none of them answers undeclared methods.
fn fully_known(index: &Index, class: &str, check_magic: bool) -> bool {
    let mut seen: HashSet<String> = HashSet::new();
    let mut pending = vec![class.to_string()];
    while let Some(next) = pending.pop() {
        if !seen.insert(next.to_ascii_lowercase()) {
            continue;
        }
        if ENUM_INTERFACES.iter().any(|i| next.eq_ignore_ascii_case(i)) {
            continue;
        }
        let declarations = index.classes_named(&next);
        // The stubs give a built-in class as one PHP version has it, and the project
        // may run another, so its members are never certain.
        if declarations.is_empty() || declarations.iter().any(|d| index.is_stub(d.path)) {
            return false;
        }
        for declaration in declarations {
            // A `class_alias` has no shape: its members are unknown.
            let Some(shape) = &declaration.declaration.class else {
                return false;
            };
            if check_magic && shape.magic {
                return false;
            }
            pending.extend(shape.parent.iter().cloned());
            pending.extend(shape.interfaces.iter().cloned());
            pending.extend(shape.traits.iter().cloned());
        }
    }
    true
}

/// Whether `class`, one of its parents or one of their traits gets a method named
/// `name` from a trait `as` alias.
fn aliased(index: &Index, class: &str, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let mut seen: HashSet<String> = HashSet::new();
    let mut pending = vec![class.to_string()];
    while let Some(next) = pending.pop() {
        if !seen.insert(next.to_ascii_lowercase()) {
            continue;
        }
        for declaration in index.classes_named(&next) {
            let Some(shape) = &declaration.declaration.class else {
                continue;
            };
            if shape.aliases.contains(&name) {
                return true;
            }
            pending.extend(shape.parent.iter().cloned());
            pending.extend(shape.traits.iter().cloned());
        }
    }
    false
}

/// Whether the member name at `start` is accessed on `$this`.
fn on_this(source: &[u8], start: u32) -> bool {
    let before = source
        .get(..start as usize)
        .unwrap_or_default()
        .trim_ascii_end();
    let Some(before) = before.strip_suffix(b"->") else {
        return false;
    };
    let before = before.strip_suffix(b"?").unwrap_or(before).trim_ascii_end();
    before.ends_with(b"$this")
        && before
            .len()
            .checked_sub(6)
            .and_then(|i| before.get(i))
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_' || *b == b'$'))
}

fn is_trait(index: &Index, fqn: &str) -> bool {
    index
        .classes_named(fqn)
        .iter()
        .any(|c| c.declaration.kind == SymbolKind::Trait)
}

fn within(spans: &[(u32, u32)], offset: u32) -> bool {
    spans
        .iter()
        .any(|&(start, end)| start <= offset && offset < end)
}

/// Whether the class name ending at `end` is followed by `::class`.
fn fetches_class_name(source: &[u8], end: u32) -> bool {
    let rest = source
        .get(end as usize..)
        .unwrap_or_default()
        .trim_ascii_start();
    let Some(rest) = rest.strip_prefix(b"::") else {
        return false;
    };
    let rest = rest.trim_ascii_start();
    rest.len() >= 5
        && rest[..5].eq_ignore_ascii_case(b"class")
        && rest
            .get(5)
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
}

/// Whether the word right before `start`, skipping whitespace and block comments,
/// is `word`.
fn after_word(source: &[u8], start: u32, word: &[u8]) -> bool {
    let mut before = source
        .get(..start as usize)
        .unwrap_or_default()
        .trim_ascii_end();
    while before.ends_with(b"*/") {
        let Some(open) = before.windows(2).rposition(|w| w == b"/*") else {
            break;
        };
        before = before[..open].trim_ascii_end();
    }
    before.len() >= word.len()
        && before[before.len() - word.len()..].eq_ignore_ascii_case(word)
        && before
            .len()
            .checked_sub(word.len() + 1)
            .and_then(|i| before.get(i))
            .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
}
