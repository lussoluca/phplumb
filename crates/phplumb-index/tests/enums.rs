use std::path::{Path, PathBuf};

use phplumb_index::discover::DiscoverOptions;
use phplumb_index::summary::summarize;
use phplumb_index::{Index, Location};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/enums")
        .canonicalize()
        .unwrap()
}

fn build() -> Index {
    Index::build(&fixture(), &DiscoverOptions::default())
        .unwrap()
        .0
}

/// The file `file` of the fixture, its content, and the offset of the `nth`
/// occurrence of `needle` in it.
fn at(file: &str, needle: &str, nth: usize) -> (PathBuf, Vec<u8>, u32) {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8(source.clone()).unwrap();
    let offset = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of {needle} in {file}"))
        .0;
    (path, source, offset as u32)
}

fn definition(index: &mut Index, file: &str, needle: &str, nth: usize) -> Vec<Location> {
    let (path, source, offset) = at(file, needle, nth);
    index.definition(&path, &source, offset)
}

/// Each location as its path, relative to the stubs directory when it is a stub and
/// to the fixture otherwise, and the text at its range.
fn described(index: &Index, locations: &[Location]) -> Vec<(String, String)> {
    let stubs = index.stubs_dir().map(Path::to_path_buf);
    locations
        .iter()
        .map(|location| {
            let text = std::fs::read(&location.path).unwrap();
            let range = &text[location.start as usize..location.end as usize];
            let relative = stubs
                .as_deref()
                .and_then(|dir| location.path.strip_prefix(dir).ok())
                .or_else(|| location.path.strip_prefix(fixture()).ok())
                .unwrap_or(&location.path);
            (
                relative.display().to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect()
}

fn pair(path: &str, text: &str) -> (String, String) {
    (path.to_string(), text.to_string())
}

fn interfaces(source: &str, fqn: &str) -> Vec<String> {
    let summary = summarize(source.as_bytes());
    summary
        .declarations
        .into_iter()
        .find(|d| d.fqn == fqn)
        .and_then(|d| d.class)
        .unwrap_or_else(|| panic!("no class {fqn}"))
        .interfaces
}

#[test]
fn enum_interfaces_include_the_implicit_ones() {
    assert_eq!(
        interfaces("<?php enum S: string implements Foo {}", "S"),
        ["Foo", "UnitEnum", "BackedEnum"]
    );
    assert_eq!(interfaces("<?php enum P {}", "P"), ["UnitEnum"]);
}

#[test]
fn implicit_interfaces_are_listed_once() {
    assert_eq!(
        interfaces("<?php enum S implements \\UnitEnum {}", "S"),
        ["UnitEnum"]
    );
    assert_eq!(
        interfaces("<?php enum B: int implements \\BackedEnum {}", "B"),
        ["BackedEnum", "UnitEnum"]
    );
}

#[test]
fn backed_enum_members_open_their_stub() {
    let mut index = build();
    for (needle, name) in [
        ("from('low')", "from"),
        ("tryFrom", "tryFrom"),
        ("value;", "value"),
    ] {
        let found = definition(&mut index, "app/Uses.php", needle, 0);
        assert_eq!(
            described(&index, &found),
            [pair("Core/Core_c.php", name)],
            "{name}"
        );
    }
}

#[test]
fn unit_enum_members_open_their_stub() {
    let mut index = build();
    for (needle, name) in [("cases", "cases"), ("name;", "name")] {
        let found = definition(&mut index, "app/Uses.php", needle, 0);
        assert_eq!(
            described(&index, &found),
            [pair("Core/Core_c.php", name)],
            "{name}"
        );
    }
}

#[test]
fn pure_enums_have_no_backed_members() {
    let mut index = build();
    assert!(definition(&mut index, "app/Uses.php", "from('x')", 0).is_empty());
}

/// The `label` method declared in `app/Severity.php`.
fn label() -> [(String, String); 1] {
    [pair("app/Severity.php", "label")]
}

#[test]
fn enum_cases_have_their_enum_type() {
    let mut index = build();
    let found = definition(&mut index, "app/Uses.php", "label();", 0);
    assert_eq!(described(&index, &found), label());
}

#[test]
fn from_returns_the_enum() {
    let mut index = build();
    for nth in [1, 2] {
        let found = definition(&mut index, "app/Uses.php", "label();", nth);
        assert_eq!(described(&index, &found), label(), "label {nth}");
    }
}

#[test]
fn self_cases_resolve_inside_the_enum() {
    let mut index = build();
    let found = definition(&mut index, "app/Severity.php", "label();", 0);
    assert_eq!(described(&index, &found), label());
}

#[test]
fn class_constants_get_no_enum_type() {
    let mut index = build();
    assert!(definition(&mut index, "app/Uses.php", "label();", 3).is_empty());
}

#[test]
fn static_cases_resolve_inside_the_enum() {
    let mut index = build();
    let found = definition(&mut index, "app/Severity.php", "label();", 1);
    assert_eq!(described(&index, &found), label());
}

#[test]
fn pure_enum_cases_have_no_value() {
    let mut index = build();
    assert!(definition(&mut index, "app/Uses.php", "value; // pure", 0).is_empty());
}

#[test]
fn this_cases_resolve() {
    let mut index = build();
    let found = definition(&mut index, "app/Severity.php", "label();", 2);
    assert_eq!(described(&index, &found), label());
}

#[test]
fn variable_classes_resolve() {
    let mut index = build();
    let found = definition(&mut index, "app/Uses.php", "label();", 4);
    assert_eq!(described(&index, &found), label());
}

#[test]
fn string_class_names_give_nothing() {
    let mut index = build();
    assert!(definition(&mut index, "app/Uses.php", "label();", 5).is_empty());
}

#[test]
fn trait_self_cases_reach_the_using_enums() {
    let mut index = build();
    let found = definition(&mut index, "app/Labels.php", "label();", 0);
    assert_eq!(described(&index, &found), label());
    let case = definition(&mut index, "app/Labels.php", "Low", 0);
    assert_eq!(described(&index, &case), [pair("app/Severity.php", "Low")]);
}

#[test]
fn trait_cycles_through_duplicate_names_end() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("app")).unwrap();
    for (file, source) in [
        (
            "app/TA.php",
            "<?php\ntrait A { public function go() { return self::Low->label(); } }\n",
        ),
        ("app/CA.php", "<?php\nclass A { use B; }\n"),
        ("app/TB.php", "<?php\ntrait B {}\n"),
        ("app/CB.php", "<?php\nclass B { use A; }\n"),
    ] {
        std::fs::write(root.join(file), source).unwrap();
    }
    let (mut index, _) = Index::build(&root, &DiscoverOptions::default()).unwrap();
    let path = root.join("app/TA.php");
    let source = std::fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find("label").unwrap() as u32;

    assert!(index.definition(&path, &source, offset).is_empty());
}
