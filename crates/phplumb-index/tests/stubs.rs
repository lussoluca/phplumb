use std::path::{Path, PathBuf};

use phplumb_index::discover::DiscoverOptions;
use phplumb_index::stubs::summarize_stub;
use phplumb_index::summary::{Declaration, Member, MemberKind, SymbolKind, summarize};
use phplumb_index::{Index, Location};

const HEADER: &str = "<?php\n\
use JetBrains\\PhpStorm\\Internal\\LanguageLevelTypeAware;\n\
use JetBrains\\PhpStorm\\Internal\\PhpStormStubsElementAvailable;\n";

fn stub(body: &str) -> String {
    format!("{HEADER}{body}")
}

fn class<'a>(declarations: &'a [Declaration], fqn: &str) -> &'a Declaration {
    declarations
        .iter()
        .find(|d| d.fqn == fqn && d.class.is_some())
        .unwrap_or_else(|| panic!("no class {fqn}"))
}

fn methods<'a>(declaration: &'a Declaration, name: &str) -> Vec<&'a Member> {
    declaration
        .class
        .as_ref()
        .unwrap()
        .members
        .iter()
        .filter(|m| m.kind == MemberKind::Method && m.name == name)
        .collect()
}

/// The `ty` that `summarize` gives a method `m` declared with `returns`.
fn declared_ty(returns: &str) -> Option<phplumb_index::phpdoc::TypeExpr> {
    let source = format!("<?php\nclass T {{ public function m(): {returns} {{}} }}\n");
    let summary = summarize(source.as_bytes());
    methods(class(&summary.declarations, "T"), "m")[0]
        .ty
        .clone()
}

#[test]
fn stub_members_keep_the_php_85_declaration() {
    let source = stub(
        "class D {\n\
         #[PhpStormStubsElementAvailable(from: '5.5', to: '8.2')]\n\
         public function modify($m) {}\n\
         #[PhpStormStubsElementAvailable(from: '8.3')]\n\
         public function modify($m) {}\n\
         #[PhpStormStubsElementAvailable(to: '7.4')]\n\
         public function gone() {}\n\
         }\n",
    );
    let second = source.rfind("modify").unwrap() as u32;
    let summary = summarize_stub(source.as_bytes());
    let d = class(&summary.declarations, "D");
    let modify = methods(d, "modify");
    assert_eq!(modify.len(), 1);
    assert_eq!(modify[0].name_start, second);
    assert!(methods(d, "gone").is_empty());
}

#[test]
fn stub_functions_keep_the_php_85_declaration() {
    let source = stub(
        "#[PhpStormStubsElementAvailable(from: '8.0')]\n\
         function f(): int {}\n\
         #[PhpStormStubsElementAvailable(from: '5.3', to: '7.4')]\n\
         function f() {}\n",
    );
    let first = source.find("function f").unwrap() as u32 + "function ".len() as u32;
    let summary = summarize_stub(source.as_bytes());
    let found: Vec<&Declaration> = summary
        .declarations
        .iter()
        .filter(|d| d.kind == SymbolKind::Function && d.fqn == "f")
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name_start, first);
}

#[test]
fn positional_availability_bounds_count() {
    let source = stub(
        "#[PhpStormStubsElementAvailable('8.6')]\n\
         function h() {}\n\
         function kept() {}\n",
    );
    let summary = summarize_stub(source.as_bytes());
    let names: Vec<&str> = summary
        .declarations
        .iter()
        .map(|d| d.fqn.as_str())
        .collect();
    assert_eq!(names, ["kept"]);
}

#[test]
fn language_level_types_pick_the_newest_version_up_to_85() {
    let source = stub(
        "class D {}\nclass E {}\nclass C {\n\
         #[LanguageLevelTypeAware(['8.4' => 'D', '8.6' => 'E'], default: 'static|false')]\n\
         public function m() {}\n\
         }\n",
    );
    let summary = summarize_stub(source.as_bytes());
    let m = methods(class(&summary.declarations, "C"), "m");
    assert_eq!(m[0].ty, declared_ty("D"));
}

#[test]
fn language_level_types_fall_back_to_default() {
    let source = stub(
        "class D {}\nclass E {}\nclass C {\n\
         #[LanguageLevelTypeAware(['8.6' => 'E'], default: 'D')]\n\
         public function m() {}\n\
         }\n",
    );
    let summary = summarize_stub(source.as_bytes());
    let m = methods(class(&summary.declarations, "C"), "m");
    assert_eq!(m[0].ty, declared_ty("D"));
}

#[test]
fn unmarked_stub_declarations_stay() {
    let source = stub("class K {\npublic function run(): K {}\n}\nfunction g(): K {}\n");
    let stubbed = summarize_stub(source.as_bytes());
    let plain = summarize(source.as_bytes());
    let shape = |summary: &[Declaration]| -> Vec<(String, u32, Option<String>)> {
        summary
            .iter()
            .flat_map(|d| {
                let own = (
                    d.fqn.clone(),
                    d.name_start,
                    format!("{:?}", d.returns).into(),
                );
                let members = d
                    .class
                    .iter()
                    .flat_map(|c| c.members.iter())
                    .map(|m| (m.name.clone(), m.name_start, format!("{:?}", m.ty).into()));
                std::iter::once(own).chain(members).collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(shape(&stubbed.declarations), shape(&plain.declarations));
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stubs")
        .canonicalize()
        .unwrap()
}

fn build(opts: &DiscoverOptions) -> Index {
    Index::build(&fixture(), opts).unwrap().0
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

fn under_stubs(index: &Index, location: &Location) -> bool {
    index
        .stubs_dir()
        .is_some_and(|dir| location.path.starts_with(dir))
}

/// A vendor polyfill of the built-in `Stringable`.
const POLYFILL: &str = "vendor/symfony/polyfill-php80/Resources/stubs/Stringable.php";

fn pair(path: &str, text: &str) -> (String, String) {
    (path.to_string(), text.to_string())
}

#[test]
fn built_in_method_opens_its_stub() {
    let mut index = build(&DiscoverOptions::default());
    let found = definition(&mut index, "app/Uses.php", "getMessage", 0);
    assert_eq!(
        described(&index, &found),
        [pair("Core/Core_c.php", "getMessage")]
    );
}

#[test]
fn inherited_built_in_members_resolve() {
    let mut index = build(&DiscoverOptions::default());
    let direct = definition(&mut index, "app/Uses.php", "getMessage", 0);
    let inherited = definition(&mut index, "app/Uses.php", "getMessage", 1);
    assert_eq!(inherited, direct);
    let constructor = definition(&mut index, "app/Uses.php", "__construct('failed')", 0);
    assert_eq!(
        described(&index, &constructor),
        [pair("Core/Core_c.php", "__construct")]
    );
}

#[test]
fn built_in_return_types_continue_the_chain() {
    let mut index = build(&DiscoverOptions::default());
    let found = definition(&mut index, "app/Uses.php", "format", 0);
    assert_eq!(
        described(&index, &found),
        [pair("date/date_c.php", "format")]
    );
}

#[test]
fn version_duplicates_give_one_target() {
    let mut index = build(&DiscoverOptions::default());
    let found = definition(&mut index, "app/Uses.php", "modify", 0);
    assert_eq!(found.len(), 1);
    let text = std::fs::read_to_string(&found[0].path).unwrap();
    let before = &text[..found[0].start as usize];
    let attribute = before
        .rfind("PhpStormStubsElementAvailable(")
        .expect("modify has an availability attribute");
    assert!(before[attribute..].starts_with("PhpStormStubsElementAvailable(from: '8.3')"));
}

#[test]
fn built_in_function_and_constant_open_stubs() {
    let mut index = build(&DiscoverOptions::default());
    for (needle, name) in [("count([])", "count"), ("PHP_EOL", "PHP_EOL")] {
        let found = definition(&mut index, "app/Uses.php", needle, 0);
        assert_eq!(found.len(), 1, "{name}");
        assert!(under_stubs(&index, &found[0]), "{name}");
        assert_eq!(described(&index, &found)[0].1, name);
    }
}

#[test]
fn project_function_wins_in_its_namespace() {
    let mut index = build(&DiscoverOptions::default());
    let found = definition(&mut index, "app/Local.php", "count()", 1);
    assert_eq!(described(&index, &found), [pair("app/Local.php", "count")]);
}

#[test]
fn polyfill_of_a_built_in_class_is_not_a_target() {
    let mut index = build(&DiscoverOptions::default());
    let found = definition(&mut index, "app/Uses.php", "Stringable", 0);
    assert_eq!(found.len(), 1);
    assert!(under_stubs(&index, &found[0]));
}

#[test]
fn hover_shows_the_stub_signature() {
    let mut index = build(&DiscoverOptions::default());
    let (path, source, offset) = at("app/Uses.php", "getMessage", 0);
    let hover = index.hover(&path, &source, offset).expect("a hover");
    assert!(
        hover.markdown.contains("getMessage(): string"),
        "{}",
        hover.markdown
    );
}

#[test]
fn stub_files_never_enter_the_project_index() {
    let mut index = build(&DiscoverOptions::default());
    for (needle, nth) in [
        ("getMessage", 0),
        ("format", 0),
        ("count([])", 0),
        ("Stringable", 0),
    ] {
        definition(&mut index, "app/Uses.php", needle, nth);
    }
    let stubs = index.stubs_dir().expect("stubs are loaded").to_path_buf();
    assert!(
        index
            .declarations()
            .all(|(path, _)| !path.starts_with(&stubs))
    );
}

#[test]
fn without_stubs_built_ins_resolve_to_nothing() {
    let opts = DiscoverOptions {
        stubs: None,
        ..DiscoverOptions::default()
    };
    let mut index = build(&opts);
    assert!(definition(&mut index, "app/Uses.php", "getMessage", 0).is_empty());
    assert!(definition(&mut index, "app/Uses.php", "count([])", 0).is_empty());
    let found = definition(&mut index, "app/Uses.php", "Stringable", 0);
    assert_eq!(described(&index, &found), [pair(POLYFILL, "Stringable")]);
}

#[test]
fn references_of_a_built_in_are_empty() {
    let mut index = build(&DiscoverOptions::default());
    let (path, source, offset) = at("app/Uses.php", "getMessage", 0);
    assert!(index.references(&path, &source, offset, false).is_empty());
    assert!(index.references(&path, &source, offset, true).is_empty());
}

#[test]
fn implementations_leave_stubs_out() {
    let mut index = build(&DiscoverOptions::default());
    let (path, source, offset) = at("app/Uses.php", "RuntimeException", 1);
    let found = index.implementations(&path, &source, offset);
    assert!(
        found.iter().all(|location| !under_stubs(&index, location)),
        "{found:?}"
    );
}

#[test]
fn implementations_of_built_in_calls_leave_stubs_out() {
    let mut index = build(&DiscoverOptions::default());
    for nth in [0, 1] {
        let (path, source, offset) = at("app/Uses.php", "getMessage", nth);
        let found = index.implementations(&path, &source, offset);
        assert!(
            found.iter().all(|location| !under_stubs(&index, location)),
            "{found:?}"
        );
    }
}

#[test]
fn built_in_classes_declared_twice_give_one_target() {
    let mut index = build(&DiscoverOptions::default());
    for needle in ["Map", "Cluster", "get("] {
        let found = definition(&mut index, "app/Twice.php", needle, 0);
        assert_eq!(found.len(), 1, "{needle}: {found:?}");
    }
}

#[test]
fn project_class_with_a_built_in_name_wins() {
    let mut index = build(&DiscoverOptions::default());
    let class = definition(&mut index, "app/Legacy.php", "Worker", 0);
    assert_eq!(
        described(&index, &class),
        [pair("lib/Worker.php", "Worker")]
    );
    let method = definition(&mut index, "app/Legacy.php", "work()", 0);
    assert_eq!(described(&index, &method), [pair("lib/Worker.php", "work")]);
}

/// A project whose `uses.php` creates `class`, indexed with the stubs in `stubs`;
/// the definition of `class` there.
fn stub_definition(stubs: &Path, class: &str) -> Vec<PathBuf> {
    let project = tempfile::TempDir::new().unwrap();
    let root = project.path().canonicalize().unwrap();
    let uses = root.join("uses.php");
    let source = format!("<?php\nnew \\{class}();\n");
    std::fs::write(&uses, &source).unwrap();
    let opts = DiscoverOptions {
        stubs: Some(stubs.to_path_buf()),
        ..DiscoverOptions::default()
    };
    let (mut index, _) = Index::build(&root, &opts).unwrap();
    let offset = source.find(class).unwrap() as u32;
    index
        .definition(&uses, source.as_bytes(), offset)
        .into_iter()
        .map(|location| location.path)
        .collect()
}

#[cfg(unix)]
#[test]
fn unreadable_stub_directories_skip_only_their_files() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let stubs = dir.path().canonicalize().unwrap();
    std::fs::write(stubs.join("a.php"), "<?php\nclass ArrayObject {}\n").unwrap();
    let locked = stubs.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("b.php"), "<?php\nclass ArrayIterator {}\n").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Permissions do not stop a privileged user.
    let readable = std::fs::read_dir(&locked).is_ok();

    let found = (!readable).then(|| stub_definition(&stubs, "ArrayObject"));

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if let Some(found) = found {
        assert_eq!(found, vec![stubs.join("a.php")]);
    }
}

#[test]
fn an_empty_stub_directory_is_read_again() {
    let dir = tempfile::TempDir::new().unwrap();
    let stubs = dir.path().canonicalize().unwrap();
    assert!(stub_definition(&stubs, "ArrayObject").is_empty());

    std::fs::write(stubs.join("x.php"), "<?php\nclass ArrayObject {}\n").unwrap();

    assert_eq!(
        stub_definition(&stubs, "ArrayObject"),
        vec![stubs.join("x.php")]
    );
}

#[test]
fn polyfill_references_are_empty() {
    let mut index = build(&DiscoverOptions::default());
    let (path, source, offset) = at(POLYFILL, "Stringable", 0);

    assert!(index.references(&path, &source, offset, false).is_empty());
}

#[test]
fn project_classes_with_built_in_names_keep_their_references() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let declaration = root.join("src/Stringable.php");
    let source = "<?php\nclass Stringable {}\n";
    std::fs::write(&declaration, source).unwrap();
    std::fs::write(
        root.join("src/Uses.php"),
        "<?php\nfunction uses(\\Stringable $s): void {}\n",
    )
    .unwrap();
    let (mut index, _) = Index::build(&root, &DiscoverOptions::default()).unwrap();
    let offset = source.find("Stringable").unwrap() as u32;

    let found = index.references(&declaration, source.as_bytes(), offset, false);

    let paths: Vec<PathBuf> = found.into_iter().map(|l| l.path).collect();
    assert_eq!(paths, vec![root.join("src/Uses.php")]);
}

#[test]
fn hover_shows_language_level_types() {
    let mut index = build(&DiscoverOptions::default());
    let (path, source, offset) = at("app/Uses.php", "modify", 0);
    let hover = index.hover(&path, &source, offset).expect("a hover");
    assert!(
        hover
            .markdown
            .contains("DateTimeImmutable::modify(string $modifier): DateTimeImmutable"),
        "{}",
        hover.markdown
    );
    assert!(!hover.markdown.contains("#["), "{}", hover.markdown);
}

#[test]
fn stub_signatures_keep_one_type() {
    let source = stub(
        "class C {\n\
         public function f(#[LanguageLevelTypeAware(['8.0' => 'int'], default: '')] int $a, #[\\SensitiveParameter] $b) {}\n\
         }\n",
    );
    let summary = summarize_stub(source.as_bytes());
    let f = methods(class(&summary.declarations, "C"), "f")[0];
    assert_eq!(f.signature, "public function f(int $a, $b)");
}

#[test]
fn stub_functions_show_language_level_types() {
    let source = stub(
        "#[LanguageLevelTypeAware(['8.0' => 'string'], default: 'string|false')]\n\
         function g(#[LanguageLevelTypeAware(['8.0' => 'array'], default: '')] $list) {}\n",
    );
    let summary = summarize_stub(source.as_bytes());
    let g = summary
        .declarations
        .iter()
        .find(|d| d.fqn == "g")
        .expect("function g");
    assert_eq!(g.signature, "function g(array $list): string");
}
