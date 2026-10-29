//! References: candidate files from the key table, kept only where an occurrence
//! resolves to the symbol.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::calls::after_new;
use crate::index::{Index, Location};
use crate::infer::{self, Receiver};
use crate::summary::{ClassSel, FileSummary, MemberKind, NameKind, NameRef, RefKey, summarize};

/// A symbol whose references are searched.
pub(crate) enum Symbol {
    Class(String),
    Function(String),
    Constant(String),
    /// A member: its own declaration, the declarations calls may reach it through
    /// (those it overrides or implements, each labeled `Class::member`), and, for a
    /// constructor, the classes `new` builds with it.
    Member {
        name: String,
        kind: MemberKind,
        own: Location,
        family: Vec<(Location, String)>,
        constructed: Vec<String>,
    },
}

impl Symbol {
    fn keys(&self) -> Vec<RefKey> {
        match self {
            Symbol::Class(fqn) => vec![
                RefKey::Class(fqn.clone()),
                RefKey::Annotation(fqn.rsplit('\\').next().unwrap_or(fqn).to_string()),
            ],
            Symbol::Function(fqn) => vec![RefKey::Function(fqn.clone())],
            Symbol::Constant(fqn) => vec![RefKey::Constant(fqn.clone())],
            Symbol::Member {
                name, constructed, ..
            } => std::iter::once(RefKey::Member(name.clone()))
                .chain(constructed.iter().map(|fqn| RefKey::Class(fqn.clone())))
                .collect(),
        }
    }
}

/// An occurrence of a symbol. `via` labels a member reached through a declaration
/// it overrides or implements.
pub(crate) struct Found {
    pub location: Location,
    pub via: Option<String>,
}

/// References of `symbol` across its candidate files, scanned in parallel.
pub(crate) fn scan(index: &Index, symbol: &Symbol) -> Vec<Location> {
    scan_with(index, symbol, |_, _, _, found| {
        found.into_iter().map(|f| f.location).collect()
    })
}

/// Runs `each` on every candidate file of `symbol` that has occurrences, in
/// parallel, with the file's source and summary.
pub(crate) fn scan_with<R: Send>(
    index: &Index,
    symbol: &Symbol,
    each: impl Fn(&Path, &[u8], &FileSummary, Vec<Found>) -> Vec<R> + Sync,
) -> Vec<R> {
    let mut files: Vec<PathBuf> = Vec::new();
    for key in symbol.keys() {
        files.extend(
            index
                .candidate_files(&key)
                .into_iter()
                .map(Path::to_path_buf),
        );
    }
    files.sort();
    files.dedup();
    files
        .par_iter()
        .flat_map_iter(|path| {
            let Ok(source) = std::fs::read(path) else {
                return Vec::new();
            };
            let summary = summarize(&source);
            let found = scan_file(index, path, &source, &summary, symbol);
            if found.is_empty() {
                return Vec::new();
            }
            each(path, &source, &summary, found)
        })
        .collect()
}

fn scan_file(
    index: &Index,
    path: &Path,
    source: &[u8],
    summary: &FileSummary,
    symbol: &Symbol,
) -> Vec<Found> {
    let at = |start: u32, end: u32| Found {
        location: Location {
            path: path.to_path_buf(),
            start,
            end,
        },
        via: None,
    };
    let names = |kind: NameKind, matches: &dyn Fn(&NameRef) -> bool| -> Vec<Found> {
        summary
            .names
            .iter()
            .chain(&summary.doc_names)
            .filter(|n| n.kind == kind && matches(n))
            .map(|n| at(n.start, n.end))
            .collect()
    };
    match symbol {
        Symbol::Class(fqn) => {
            let mut found = names(NameKind::ClassLike, &|n| n.fqn.eq_ignore_ascii_case(fqn));
            found.extend(
                summary
                    .annotation_names
                    .iter()
                    .filter(|n| {
                        index
                            .annotation_classes(path, summary, n)
                            .iter()
                            .any(|class| class.eq_ignore_ascii_case(fqn))
                    })
                    .map(|n| at(n.start, n.end)),
            );
            found
        }
        Symbol::Function(fqn) => names(NameKind::Function, &|n| index.function_matches(n, fqn)),
        Symbol::Constant(fqn) => names(NameKind::Constant, &|n| index.constant_matches(n, fqn)),
        Symbol::Member {
            name,
            kind,
            own,
            family,
            constructed,
        } => {
            // Whether a dispatched occurrence reaches `own`, and through what.
            let reached = |targets: &[(Location, String)]| -> Option<Option<String>> {
                if targets.iter().any(|(target, _)| target == own) {
                    return Some(None);
                }
                targets.iter().find_map(|(target, _)| {
                    family
                        .iter()
                        .find(|(member, _)| member == target)
                        .map(|(_, label)| Some(label.clone()))
                })
            };
            let mut found = Vec::new();
            for static_ref in &summary.static_refs {
                if static_ref.kind != *kind || !same_name(&static_ref.name, name, *kind) {
                    continue;
                }
                let targets = static_targets(index, path, summary, static_ref, *kind, name);
                // Only `static::` dispatches; `self::`, `parent::` and a named class
                // run the one declaration they resolve to.
                let via = match static_ref.class {
                    ClassSel::StaticKw => reached(&targets),
                    _ => targets
                        .iter()
                        .any(|(target, _)| target == own)
                        .then_some(None),
                };
                if let Some(via) = via {
                    found.push(Found {
                        via,
                        ..at(static_ref.start, static_ref.end)
                    });
                }
            }
            for doc in &summary.doc_members {
                if !same_name(&doc.name, name, *kind) {
                    continue;
                }
                let as_static = |kind: MemberKind| crate::summary::StaticRef {
                    kind,
                    name: doc.name.clone(),
                    start: doc.start,
                    end: doc.end,
                    class: doc.class.clone(),
                };
                // `Foo::NAME` names a constant of that exact name, else a method.
                let kind_matches = match (doc.kind, *kind) {
                    (Some(doc_kind), kind) => doc_kind == kind,
                    (None, MemberKind::Constant) => true,
                    (None, MemberKind::Method) => static_targets(
                        index,
                        path,
                        summary,
                        &as_static(MemberKind::Constant),
                        MemberKind::Constant,
                        &doc.name,
                    )
                    .is_empty(),
                    (None, _) => false,
                };
                if !kind_matches {
                    continue;
                }
                let targets = static_targets(index, path, summary, &as_static(*kind), *kind, name);
                if targets.iter().any(|(target, _)| target == own) {
                    found.push(at(doc.start, doc.end));
                }
            }
            for (start, end, receiver, access_kind) in
                infer::member_accesses_named(index, path, summary, source, name)
            {
                if access_kind != *kind {
                    continue;
                }
                let targets = match receiver {
                    Receiver::Classes(classes) => index.member_targets(&classes, *kind, name),
                    Receiver::Anonymous(i) => {
                        index.anonymous_member_targets(path, &summary.class_ranges[i], *kind, name)
                    }
                };
                if let Some(via) = reached(&targets) {
                    found.push(Found {
                        via,
                        ..at(start, end)
                    });
                }
            }
            if !constructed.is_empty() {
                found.extend(constructions(summary, source, constructed).map(|(s, e)| at(s, e)));
            }
            found
        }
    }
}

/// Ranges of the class names and keywords after `new` that build one of `classes`.
fn constructions<'a>(
    summary: &'a FileSummary,
    source: &'a [u8],
    classes: &'a [String],
) -> impl Iterator<Item = (u32, u32)> + 'a {
    let builds = |fqn: &str| classes.iter().any(|c| c.eq_ignore_ascii_case(fqn));
    let named = summary
        .names
        .iter()
        .filter(move |n| {
            n.kind == NameKind::ClassLike && after_new(source, n.start) && builds(&n.fqn)
        })
        .map(|n| (n.start, n.end));
    let keywords = summary
        .keywords
        .iter()
        .filter(move |k| {
            after_new(source, k.start) && summary.keyword_class(k).is_some_and(|fqn| builds(&fqn))
        })
        .map(|k| (k.start, k.end));
    named.chain(keywords)
}

fn static_targets(
    index: &Index,
    path: &Path,
    summary: &FileSummary,
    static_ref: &crate::summary::StaticRef,
    kind: MemberKind,
    name: &str,
) -> Vec<(Location, String)> {
    let enclosing = summary.enclosing_class(static_ref.start);
    let class = match &static_ref.class {
        ClassSel::Named(fqn) => Some(fqn.clone()),
        ClassSel::SelfKw | ClassSel::StaticKw => match enclosing {
            Some(range) if range.fqn.is_none() => {
                return index.anonymous_member_targets(path, range, kind, name);
            }
            Some(range) => range.fqn.clone(),
            None => None,
        },
        ClassSel::ParentKw => enclosing.and_then(|range| range.parent.clone()),
    };
    class
        .map(|class| index.member_targets(&[class], kind, name))
        .unwrap_or_default()
}

/// Methods match case-insensitively; properties and constants exactly.
pub(crate) fn same_name(found: &str, wanted: &str, kind: MemberKind) -> bool {
    match kind {
        MemberKind::Method => found.eq_ignore_ascii_case(wanted),
        _ => found == wanted,
    }
}
