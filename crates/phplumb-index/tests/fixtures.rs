//! Runs the marker-annotated fixture workspaces under `tests/fixtures`.
//!
//! `/*@cursor:ID*/` places a cursor at the byte right after the marker.
//! `/*@target:ID*/` marks a target at the first byte after the marker that is neither
//! whitespace nor another marker; on `$name` or a quoted name it marks the name itself. A cursor passes only when the set of results equals
//! the set of targets with the same ID; a cursor with no target expects nothing.
//! Several cursors may share an ID.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use phplumb_index::Location;
use phplumb_index::discover::{DiscoverOptions, discover};
use phplumb_index::index::Index;

const OPEN: &[u8] = b"/*@";
const CLOSE: &[u8] = b"*/";

struct Marker {
    kind: String,
    id: String,
    end: usize,
}

fn markers(text: &[u8]) -> Vec<Marker> {
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(rel) = find(&text[pos..], OPEN) {
        let start = pos + rel + OPEN.len();
        let close = start + find(&text[start..], CLOSE).expect("unterminated marker");
        let body = std::str::from_utf8(&text[start..close]).unwrap();
        let (kind, id) = body.split_once(':').expect("marker without id");
        found.push(Marker {
            kind: kind.to_string(),
            id: id.to_string(),
            end: close + CLOSE.len(),
        });
        pos = close + CLOSE.len();
    }
    found
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// First byte at or after `pos` that is neither whitespace nor inside a marker.
fn skip_to_code(text: &[u8], mut pos: usize) -> usize {
    loop {
        while pos < text.len() && text[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if text[pos..].starts_with(OPEN) {
            pos += find(&text[pos..], CLOSE).unwrap() + CLOSE.len();
        } else if matches!(text.get(pos), Some(b'$' | b'\'' | b'"')) {
            return pos + 1;
        } else {
            return pos;
        }
    }
}

fn line_of(text: &[u8], offset: usize) -> usize {
    text[..offset].iter().filter(|b| **b == b'\n').count() + 1
}

fn run_fixture(name: &str) {
    run_with(name, |index, path, source, offset| {
        index.definition(path, source, offset)
    });
}

fn run_implementation_fixture(name: &str) {
    run_with(name, |index, path, source, offset| {
        index.implementations(path, source, offset)
    });
}

fn run_reference_fixture(name: &str) {
    run_with(name, |index, path, source, offset| {
        index.references(path, source, offset, false)
    });
}

fn run_with(name: &str, query: impl Fn(&mut Index, &Path, &[u8], u32) -> Vec<Location>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .canonicalize()
        .unwrap();
    let (mut index, _) = Index::build(&root, &DiscoverOptions::default()).unwrap();
    let rel = |path: &Path| path.strip_prefix(&root).unwrap().display().to_string();

    let mut cursors: Vec<(String, PathBuf, usize)> = Vec::new();
    let mut targets: HashMap<String, BTreeSet<(String, u32)>> = HashMap::new();
    for path in discover(&root, &DiscoverOptions::default()).unwrap() {
        let text = fs::read(&path).unwrap();
        for marker in markers(&text) {
            match marker.kind.as_str() {
                "cursor" => cursors.push((marker.id, path.clone(), marker.end)),
                "target" => {
                    let at = skip_to_code(&text, marker.end) as u32;
                    targets
                        .entry(marker.id)
                        .or_default()
                        .insert((rel(&path), at));
                }
                other => panic!("unknown marker kind {other}"),
            }
        }
    }
    assert!(!cursors.is_empty(), "fixture {name} has no cursors");

    let mut failures = Vec::new();
    for (id, path, offset) in &cursors {
        let source = fs::read(path).unwrap();
        let actual: BTreeSet<(String, u32)> = query(&mut index, path, &source, *offset as u32)
            .into_iter()
            .map(|loc| (rel(&loc.path), loc.start))
            .collect();
        let expected = targets.get(id).cloned().unwrap_or_default();
        if actual != expected {
            failures.push(format!(
                "{}:{} cursor {id}: expected {expected:?}, got {actual:?}",
                rel(path),
                line_of(&source, *offset)
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn calls() {
    run_fixture("calls");
}

#[test]
fn names() {
    run_fixture("names");
}

#[test]
fn functions() {
    run_fixture("functions");
}

#[test]
fn kinds() {
    run_fixture("kinds");
}

#[test]
fn ambiguous() {
    run_fixture("ambiguous");
}

#[test]
fn tolerant() {
    run_fixture("tolerant");
}

#[test]
fn static_members() {
    run_fixture("static_members");
}

#[test]
fn inference() {
    run_fixture("inference");
}

#[test]
fn scopes() {
    run_fixture("scopes");
}

#[test]
fn implementations() {
    run_implementation_fixture("implementations");
}

#[test]
fn inheritdoc() {
    run_fixture("inheritdoc");
}

#[test]
fn references() {
    run_reference_fixture("references");
}

/// `/*@broken*/` marks a name that must be flagged, at the first code byte after
/// the marker; the flagged names of every file must equal the marked ones.
const BROKEN: &[u8] = b"/*@broken*/";

fn diagnostics_of(name: &str) -> (PathBuf, Index) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .canonicalize()
        .unwrap();
    let (index, _) = Index::build(&root, &DiscoverOptions::default()).unwrap();
    (root, index)
}

fn run_diagnostics_fixture(name: &str) {
    let (root, index) = diagnostics_of(name);
    let mut failures = Vec::new();
    for path in discover(&root, &DiscoverOptions::default()).unwrap() {
        let source = fs::read(&path).unwrap();
        let rel = path.strip_prefix(&root).unwrap().display().to_string();
        let mut expected = BTreeSet::new();
        let mut pos = 0;
        while let Some(found) = find(&source[pos..], BROKEN) {
            let end = pos + found + BROKEN.len();
            expected.insert(skip_to_code(&source, end) as u32);
            pos = end;
        }
        let found = index.diagnostics(&path, &source);
        let actual: BTreeSet<u32> = found.iter().map(|d| d.start).collect();
        for missing in expected.difference(&actual) {
            failures.push(format!(
                "{rel}:{} not flagged",
                line_of(&source, *missing as usize)
            ));
        }
        for d in found.iter().filter(|d| !expected.contains(&d.start)) {
            failures.push(format!(
                "{rel}:{} flagged without a marker: {}",
                line_of(&source, d.start as usize),
                d.message
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn diagnostics() {
    run_diagnostics_fixture("diagnostics");
}

#[test]
fn diagnostic_messages_and_severities() {
    use phplumb_index::diagnostics::Severity;

    let (root, index) = diagnostics_of("diagnostics");
    let mut seen: Vec<(String, Severity)> = Vec::new();
    for file in ["app/Uses.php", "app/Members.php"] {
        let path = root.join(file);
        let source = fs::read(&path).unwrap();
        seen.extend(
            index
                .diagnostics(&path, &source)
                .into_iter()
                .map(|d| (d.message, d.severity)),
        );
    }
    for wanted in [
        ("Undefined class 'App\\Ghost'", Severity::Error),
        ("Undefined class 'App\\Undefined'", Severity::Warning),
        ("Undefined class 'App\\Phantom'", Severity::Warning),
        ("Undefined class 'App\\Absent'", Severity::Error),
        ("Undefined function 'App\\invented'", Severity::Error),
        ("Undefined constant 'App\\UNDEFINED_LIMIT'", Severity::Error),
        (
            "Undefined method 'Lib\\Content::invented()'",
            Severity::Error,
        ),
        (
            "Undefined class constant 'Lib\\Content::KIND'",
            Severity::Error,
        ),
    ] {
        assert!(
            seen.contains(&(wanted.0.to_string(), wanted.1)),
            "{wanted:?} not in {seen:#?}"
        );
    }
}

#[test]
fn diagnostics_stay_silent_in_namespaces_hidden_by_exclusions() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "tools/big/Heavy.php",
        "<?php\nnamespace Tools\\Big;\nclass Heavy {}\nfunction heavy_fn() {}\n",
    );
    write(
        "src/Use.php",
        "<?php\nnamespace App;\nuse Tools\\Big\\Heavy;\nclass Known {}\nnew Heavy();\n\\Tools\\Big\\heavy_fn();\nnew \\App\\Ghost();\n",
    );
    let opts = DiscoverOptions {
        exclude: vec!["tools/big/**".to_string()],
        ..DiscoverOptions::default()
    };
    let (index, _) = Index::build(&root, &opts).unwrap();
    let path = root.join("src/Use.php");
    let source = fs::read(&path).unwrap();

    let messages: Vec<String> = index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect();

    assert_eq!(messages, vec!["Undefined class 'App\\Ghost'"]);
}

#[test]
fn drupal() {
    run_fixture("drupal");
}
