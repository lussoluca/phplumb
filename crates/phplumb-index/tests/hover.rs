use std::fs;
use std::path::{Path, PathBuf};

use phplumb_index::Index;
use phplumb_index::discover::DiscoverOptions;
use tempfile::TempDir;

const FOO: &str = "<?php\nnamespace Lib;\n\n/**\n * A foo.\n */\nfinal class Foo\n{\n    /**\n     * Loads one.\n     *\n     * @return static|null\n     */\n    public function load(int $id): ?static\n    {\n    }\n}\n";
const USE: &str =
    "<?php\nnamespace App;\n\nuse Lib\\Foo;\n\nfunction f(Foo $foo)\n{\n    $foo->load(1);   \n}\n";

fn setup() -> (TempDir, PathBuf, Index) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("lib/Foo.php"), FOO).unwrap();
    fs::write(root.join("Use.php"), USE).unwrap();
    let index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    (dir, root, index)
}

fn hover_at(index: &mut Index, root: &Path, needle: &str, shift: usize) -> Option<String> {
    let path = root.join("Use.php");
    let source = fs::read(&path).unwrap();
    let offset = USE.find(needle).unwrap() + shift;
    index
        .hover(&path, &source, offset as u32)
        .map(|hover| hover.markdown)
}

#[test]
fn hover_on_a_method_shows_class_signature_and_summary() {
    let (_dir, root, mut index) = setup();

    let markdown = hover_at(&mut index, &root, "load(1)", 1).unwrap();

    assert!(
        markdown.contains("Lib\\Foo::load(int $id): ?static"),
        "{markdown}"
    );
    assert!(markdown.contains("Loads one."), "{markdown}");
}

#[test]
fn hover_on_a_class_shows_its_signature() {
    let (_dir, root, mut index) = setup();

    let markdown = hover_at(&mut index, &root, "Foo $foo", 0).unwrap();

    assert!(markdown.contains("final class Foo"), "{markdown}");
    assert!(markdown.contains("A foo."), "{markdown}");
}

#[test]
fn hover_on_a_variable_shows_its_inferred_type() {
    let (_dir, root, mut index) = setup();

    let markdown = hover_at(&mut index, &root, "$foo->", 1).unwrap();

    assert!(markdown.contains("\\Lib\\Foo $foo"), "{markdown}");
}

#[test]
fn hover_on_whitespace_is_none() {
    let (_dir, root, mut index) = setup();

    assert!(hover_at(&mut index, &root, "   \n}", 1).is_none());
}

#[test]
fn hover_on_a_static_property_shows_the_property_not_a_local() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = "<?php\nnamespace Lib;\n\nclass Other {}\n\nclass Holder\n{\n    public static int $prop = 0;\n}\n\nfunction h(): void\n{\n    $prop = new Other();\n    echo Holder::$prop;\n}\n";
    fs::write(root.join("Holder.php"), source).unwrap();
    let mut index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let offset = source.find("Holder::$prop").unwrap() + "Holder::$".len();

    let markdown = index
        .hover(&root.join("Holder.php"), source.as_bytes(), offset as u32)
        .unwrap()
        .markdown;

    assert!(markdown.contains("Lib\\Holder::$prop"), "{markdown}");
}

#[test]
fn project_parameter_attributes_stay_in_hover() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source =
        "<?php\nfunction secret(#[\\SensitiveParameter] string $p): void {}\nsecret('x');\n";
    fs::write(root.join("Use.php"), source).unwrap();
    let mut index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let path = root.join("Use.php");
    let offset = source.rfind("secret").unwrap() as u32;

    let markdown = index
        .hover(&path, source.as_bytes(), offset)
        .unwrap()
        .markdown;

    assert!(
        markdown.contains("function secret(#[\\SensitiveParameter] string $p): void"),
        "{markdown}"
    );
}

#[test]
fn hover_shows_scalar_generic_arguments() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(
        root.join("lib/Collection.php"),
        "<?php\nnamespace Lib;\n/**\n * @template K\n * @template V\n */\nclass Collection {}\n",
    )
    .unwrap();
    let source = "<?php\nnamespace App;\nuse Lib\\Collection;\nclass Item {}\nfunction f(): void\n{\n    /** @var Collection<int, Item> $items */\n    $items = g();\n    $items;\n}\n";
    fs::write(root.join("Use.php"), source).unwrap();
    let mut index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let path = root.join("Use.php");
    let offset = source.rfind("$items").unwrap() as u32 + 1;

    let markdown = index
        .hover(&path, source.as_bytes(), offset)
        .unwrap()
        .markdown;

    assert!(
        markdown.contains("\\Lib\\Collection<int, \\App\\Item> $items"),
        "{markdown}"
    );
}
