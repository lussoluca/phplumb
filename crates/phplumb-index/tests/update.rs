use std::fs;
use std::path::{Path, PathBuf};

use phplumb_index::discover::DiscoverOptions;
use phplumb_index::{Index, Location};
use tempfile::TempDir;

const TARGET: &str = "<?php\nnamespace App;\nclass Target {}\n";
const TARGET_MOVED: &str = "<?php\nnamespace App;\n\n\n\nclass Target {}\n";
const USE: &str = "<?php\nnamespace App;\nnew Target();\n";

struct Workspace {
    _dir: TempDir,
    root: PathBuf,
}

impl Workspace {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace = Workspace { _dir: dir, root };
        for (path, content) in files {
            workspace.write(path, content);
        }
        workspace
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn index(&self, opts: &DiscoverOptions) -> Index {
        Index::build(&self.root, opts).unwrap().0
    }

    /// Definition of the first occurrence of `needle` in `file`.
    fn definition(&self, index: &mut Index, file: &str, needle: &str) -> Vec<Location> {
        let path = self.path(file);
        let source = fs::read(&path).unwrap();
        let offset = find(&source, needle.as_bytes()) as u32;
        index.definition(&path, &source, offset)
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("needle in file")
}

fn starts(locations: &[Location]) -> Vec<(PathBuf, u32)> {
    locations
        .iter()
        .map(|l| (l.path.clone(), l.start))
        .collect()
}

fn offset_of(path: &Path, needle: &str) -> u32 {
    find(&fs::read(path).unwrap(), needle.as_bytes()) as u32
}

#[test]
fn update_sees_a_moved_class() {
    let ws = Workspace::new(&[("src/A.php", TARGET), ("src/Use.php", USE)]);
    let mut index = ws.index(&DiscoverOptions::default());
    ws.write("src/A.php", TARGET_MOVED);

    let stats = index.update(&[ws.path("src/A.php")], &[]);

    assert_eq!(stats.reparsed, 1);
    assert_eq!(
        starts(&ws.definition(&mut index, "src/Use.php", "Target")),
        vec![(
            ws.path("src/A.php"),
            offset_of(&ws.path("src/A.php"), "Target")
        )]
    );
}

#[test]
fn update_adds_a_created_file() {
    let ws = Workspace::new(&[("src/Use.php", "<?php\nnamespace App;\nnew Fresh();\n")]);
    let mut index = ws.index(&DiscoverOptions::default());
    ws.write("src/New.php", "<?php\nnamespace App;\nclass Fresh {}\n");

    index.update(&[ws.path("src/New.php")], &[]);

    assert_eq!(ws.definition(&mut index, "src/Use.php", "Fresh").len(), 1);
}

#[test]
fn update_removes_a_deleted_file() {
    let ws = Workspace::new(&[("src/A.php", TARGET), ("src/Use.php", USE)]);
    let mut index = ws.index(&DiscoverOptions::default());
    fs::remove_file(ws.path("src/A.php")).unwrap();

    let stats = index.update(&[], &[ws.path("src/A.php")]);

    assert_eq!(stats.removed, 1);
    assert!(
        ws.definition(&mut index, "src/Use.php", "Target")
            .is_empty()
    );
}

#[test]
fn update_removes_a_deleted_directory() {
    let ws = Workspace::new(&[
        ("lib/A.php", TARGET),
        ("lib/B.php", "<?php\nnamespace App;\nclass Other {}\n"),
        (
            "src/Use.php",
            "<?php\nnamespace App;\nnew Target(); new Other();\n",
        ),
    ]);
    let mut index = ws.index(&DiscoverOptions::default());
    fs::remove_dir_all(ws.path("lib")).unwrap();

    let stats = index.update(&[], &[ws.path("lib")]);

    assert_eq!(stats.removed, 2);
    assert!(
        ws.definition(&mut index, "src/Use.php", "Target")
            .is_empty()
    );
    assert!(ws.definition(&mut index, "src/Use.php", "Other").is_empty());
}

#[test]
fn update_rejects_filtered_paths() {
    let ws = Workspace::new(&[
        ("vendor/composer/installed.json", r#"{"packages": []}"#),
        ("src/Use.php", "<?php\nnew X(); new Y();\n"),
    ]);
    let opts = DiscoverOptions {
        exclude: vec!["excluded/**".to_string()],
        ..DiscoverOptions::default()
    };
    let mut index = ws.index(&opts);
    ws.write("vendor/composer/abc/X.php", "<?php\nclass X {}\n");
    ws.write("excluded/Y.php", "<?php\nclass Y {}\n");

    let stats = index.update(
        &[
            ws.path("vendor/composer/abc/X.php"),
            ws.path("excluded/Y.php"),
        ],
        &[],
    );

    assert_eq!(stats.reparsed, 0);
    assert!(ws.definition(&mut index, "src/Use.php", "X").is_empty());
    assert!(ws.definition(&mut index, "src/Use.php", "Y").is_empty());
}

#[test]
fn update_treats_missing_changed_paths_as_removed() {
    let ws = Workspace::new(&[("src/A.php", TARGET), ("src/Use.php", USE)]);
    let mut index = ws.index(&DiscoverOptions::default());
    fs::remove_file(ws.path("src/A.php")).unwrap();

    let stats = index.update(&[ws.path("src/A.php")], &[]);

    assert_eq!(stats.removed, 1);
    assert!(
        ws.definition(&mut index, "src/Use.php", "Target")
            .is_empty()
    );
}

#[test]
fn update_accepts_paths_under_a_non_canonical_root() {
    let dir = TempDir::new().unwrap();
    let ws = Workspace {
        root: dir.path().canonicalize().unwrap(),
        _dir: dir,
    };
    ws.write("src/Use.php", "<?php\nnamespace App;\nnew Fresh();\n");
    let raw_root = ws._dir.path().to_path_buf();
    let mut index = Index::build(&raw_root, &DiscoverOptions::default())
        .unwrap()
        .0;
    ws.write("src/New.php", "<?php\nnamespace App;\nclass Fresh {}\n");

    index.update(&[raw_root.join("src/New.php")], &[]);

    assert_eq!(ws.definition(&mut index, "src/Use.php", "Fresh").len(), 1);
}

#[test]
fn definition_heals_a_stale_target() {
    let ws = Workspace::new(&[("src/A.php", TARGET), ("src/Use.php", USE)]);
    let mut index = ws.index(&DiscoverOptions::default());
    let original = fs::metadata(ws.path("src/A.php"))
        .unwrap()
        .modified()
        .unwrap();
    ws.write("src/A.php", TARGET_MOVED);
    fs::File::options()
        .write(true)
        .open(ws.path("src/A.php"))
        .unwrap()
        .set_modified(original)
        .unwrap();

    let found = ws.definition(&mut index, "src/Use.php", "Target");

    assert_eq!(
        starts(&found),
        vec![(
            ws.path("src/A.php"),
            offset_of(&ws.path("src/A.php"), "Target")
        )]
    );
}

#[test]
fn healing_a_file_with_two_same_name_declarations_keeps_one_entry_each() {
    let conditional = "<?php\nnamespace App;\nif (true) {\n    class Target {}\n} else {\n    class Target {}\n}\n";
    let ws = Workspace::new(&[("src/A.php", conditional), ("src/Use.php", USE)]);
    let mut index = ws.index(&DiscoverOptions::default());
    ws.write("src/A.php", &conditional.replace("<?php\n", "<?php\n\n\n"));

    let healed = ws.definition(&mut index, "src/Use.php", "Target");
    fs::remove_file(ws.path("src/A.php")).unwrap();
    index.update(&[], &[ws.path("src/A.php")]);

    assert_eq!(healed.len(), 2);
    assert!(
        ws.definition(&mut index, "src/Use.php", "Target")
            .is_empty()
    );
}

#[test]
fn many_unindexed_paths_are_handled_in_linear_time() {
    let ws = Workspace::new(&[]);
    for i in 0..3_000 {
        ws.write(
            &format!("src/m{}/C{i}.php", i % 30),
            &format!("<?php\nclass C{i} {{}}\n"),
        );
    }
    let mut index = ws.index(&DiscoverOptions::default());
    let removed: Vec<PathBuf> = (0..3_000)
        .map(|i| ws.path(&format!("templates/t{i}.html.twig")))
        .collect();
    let changed: Vec<PathBuf> = (0..3_000)
        .map(|i| ws.path(&format!("vendor/composer/abc/X{i}.php")))
        .collect();

    let started = std::time::Instant::now();
    let stats = index.update(&changed, &removed);

    assert_eq!(stats.removed, 0);
    assert!(
        started.elapsed().as_millis() < 500,
        "took {} ms",
        started.elapsed().as_millis()
    );
}

#[test]
fn update_indexes_a_renamed_directory() {
    let ws = Workspace::new(&[
        ("lib/Target.php", TARGET),
        (
            "lib/sub/Other.php",
            "<?php\nnamespace App;\nclass Other {}\n",
        ),
        (
            "src/Use.php",
            "<?php\nnamespace App;\nnew Target(); new Other();\n",
        ),
    ]);
    let mut index = ws.index(&DiscoverOptions::default());
    fs::rename(ws.path("lib"), ws.path("lib2")).unwrap();

    let stats = index.update(&[ws.path("lib2")], &[ws.path("lib")]);

    assert_eq!((stats.reparsed, stats.removed), (2, 2));
    assert_eq!(
        starts(&ws.definition(&mut index, "src/Use.php", "Target")),
        vec![(
            ws.path("lib2/Target.php"),
            offset_of(&ws.path("lib2/Target.php"), "Target")
        )]
    );
    assert_eq!(ws.definition(&mut index, "src/Use.php", "Other").len(), 1);
}

#[test]
fn update_skips_created_hidden_and_node_modules_directories() {
    let ws = Workspace::new(&[("src/Use.php", "<?php\nnew A(); new B();\n")]);
    let mut index = ws.index(&DiscoverOptions::default());
    ws.write(".cache/A.php", "<?php\nclass A {}\n");
    ws.write("node_modules/pkg/B.php", "<?php\nclass B {}\n");

    let stats = index.update(&[ws.path(".cache"), ws.path("node_modules/pkg")], &[]);

    assert_eq!(stats.reparsed, 0);
}

fn framework_files(index: &Index, root: &Path) -> Vec<String> {
    index
        .framework_files_under(root)
        .map(|path| path.strip_prefix(root).unwrap().display().to_string())
        .collect()
}

#[test]
fn framework_files_follow_updates() {
    let workspace = Workspace::new(&[
        ("a/a.services.yml", "services: {}\n"),
        ("vendor/p/p.services.yml", "services: {}\n"),
        (".hidden/h.yml", "x: 1\n"),
        ("src/A.php", "<?php\n"),
    ]);
    let mut index = workspace.index(&DiscoverOptions::default());
    assert_eq!(
        framework_files(&index, &workspace.root),
        vec!["a/a.services.yml"]
    );

    workspace.write("b/b.routing.yml", "x: {}\n");
    index.update(&[workspace.path("b")], &[]);
    assert_eq!(
        framework_files(&index, &workspace.root),
        vec!["a/a.services.yml", "b/b.routing.yml"]
    );

    fs::remove_file(workspace.path("a/a.services.yml")).unwrap();
    index.update(&[], &[workspace.path("a/a.services.yml")]);
    assert_eq!(
        framework_files(&index, &workspace.root),
        vec!["b/b.routing.yml"]
    );
    assert_eq!(
        framework_files(&index, &workspace.path("b")),
        vec!["b.routing.yml"]
    );
}
