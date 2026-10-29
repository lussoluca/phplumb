use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use phplumb_index::discover::{DiscoverOptions, discover};
use phplumb_index::frameworks::{Activation, FrameworkSettings};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn canonical_root(dir: &TempDir) -> PathBuf {
    dir.path().canonicalize().unwrap()
}

fn relative(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
        .collect()
}

const DRUPAL_EXTENSION_FILES: [&str; 9] = [
    "a.php",
    "b.module",
    "c.inc",
    "d.install",
    "e.theme",
    "f.profile",
    "g.engine",
    "h.txt",
    "i.js",
];

#[test]
fn finds_only_php_files_without_frameworks() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    for name in DRUPAL_EXTENSION_FILES {
        write(&root, name, "<?php\n");
    }

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(relative(&root, &found), vec!["a.php"]);
}

#[test]
fn finds_drupal_php_extensions_when_drupal_is_active() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    for name in DRUPAL_EXTENSION_FILES {
        write(&root, name, "<?php\n");
    }
    let opts = DiscoverOptions {
        frameworks: FrameworkSettings {
            drupal: Activation::On,
            ..FrameworkSettings::default()
        },
        ..DiscoverOptions::default()
    };

    let found = discover(&root, &opts).unwrap();

    assert_eq!(
        relative(&root, &found),
        vec![
            "a.php",
            "b.module",
            "c.inc",
            "d.install",
            "e.theme",
            "f.profile",
            "g.engine"
        ]
    );
}

#[test]
fn ignores_gitignore_rules() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    fs::create_dir_all(root.join(".git")).unwrap();
    write(&root, ".gitignore", "vendor/\n");
    write(&root, "vendor/x/A.php", "<?php\n");

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(relative(&root, &found), vec!["vendor/x/A.php"]);
}

#[test]
fn skips_hidden_dirs_and_node_modules() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, ".ds-clone/A.php", "<?php\n");
    write(&root, "node_modules/p/B.php", "<?php\n");
    write(&root, "src/C.php", "<?php\n");

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(relative(&root, &found), vec!["src/C.php"]);
}

#[test]
fn applies_exclude_globs() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "vendor/rector/rector/vendor/X.php", "<?php\n");
    write(&root, "vendor/other/Y.php", "<?php\n");
    let opts = DiscoverOptions {
        exclude: vec!["vendor/rector/rector/**".to_string()],
        ..DiscoverOptions::default()
    };

    let found = discover(&root, &opts).unwrap();

    assert_eq!(relative(&root, &found), vec!["vendor/other/Y.php"]);
}

#[test]
fn skips_files_over_size_limit() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "big.php", "<?php\n12345");
    write(&root, "ok.php", "<?php\n1234");
    let opts = DiscoverOptions {
        max_file_bytes: 10,
        ..DiscoverOptions::default()
    };

    let found = discover(&root, &opts).unwrap();

    assert_eq!(relative(&root, &found), vec!["ok.php"]);
}

#[test]
fn dedupes_symlinked_dirs() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "modules/foo/Foo.php", "<?php\n");
    fs::create_dir_all(root.join("web/modules/custom")).unwrap();
    symlink("../../../modules/foo", root.join("web/modules/custom/foo")).unwrap();

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(found, vec![root.join("modules/foo/Foo.php")]);
}

#[test]
fn dedupes_symlinks_from_outside_the_root() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    let outside = TempDir::new().unwrap();
    write(outside.path(), "foo/Foo.php", "<?php\n");
    fs::create_dir_all(root.join("web/modules/custom")).unwrap();
    for link in ["foo", "bar"] {
        symlink(
            outside.path().join("foo"),
            root.join("web/modules/custom").join(link),
        )
        .unwrap();
    }

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(
        relative(&root, &found),
        vec!["web/modules/custom/bar/Foo.php"]
    );
}

#[test]
fn dedupes_symlinked_files() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "b/B.php", "<?php\n");
    fs::create_dir_all(root.join("a")).unwrap();
    symlink("../b/B.php", root.join("a/Link.php")).unwrap();

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(relative(&root, &found), vec!["b/B.php"]);
}

#[test]
fn survives_symlink_loop() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "a/A.php", "<?php\n");
    symlink("..", root.join("a/loop")).unwrap();

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(relative(&root, &found), vec!["a/A.php"]);
}

#[test]
fn accepts_a_non_canonical_root() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/A.php", "<?php\n");

    let found = discover(dir.path(), &DiscoverOptions::default()).unwrap();

    assert_eq!(found, vec![canonical_root(&dir).join("src/A.php")]);
}

fn composer_vendor(root: &Path, installed_json: &str) {
    write(root, "vendor/composer/installed.json", installed_json);
    for rel in [
        "vendor/acme/lib/src/A.php",
        "vendor/acme/lib/functions.php",
        "vendor/acme/lib/tests/T.php",
        "vendor/acme/old/lib/Old/B.php",
        "vendor/acme/old/classes/C.php",
        "vendor/acme/tool/src/D.php",
        "vendor/composer/ClassLoader.php",
        "vendor/composer/abc123/core/E.php",
        "web/core/F.php",
    ] {
        write(root, rel, "<?php\n");
    }
}

const PACKAGES: &str = r#"[
    {"name": "acme/lib", "install-path": "../acme/lib",
     "autoload": {"psr-4": {"Acme\\": "src/"}, "files": ["functions.php"]}},
    {"name": "acme/old", "install-path": "../acme/old",
     "autoload": {"psr-0": {"Old_": ["lib/"]}, "classmap": ["classes/"]}},
    {"name": "acme/tool", "install-path": "../acme/tool", "autoload": {}}
]"#;

#[test]
fn vendor_keeps_only_composer_autoload_paths() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    composer_vendor(&root, &format!(r#"{{"packages": {PACKAGES}}}"#));

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(
        relative(&root, &found),
        vec![
            "vendor/acme/lib/functions.php",
            "vendor/acme/lib/src/A.php",
            "vendor/acme/old/classes/C.php",
            "vendor/acme/old/lib/Old/B.php",
            "vendor/composer/ClassLoader.php",
            "web/core/F.php",
        ]
    );
}

#[test]
fn vendor_reads_composer_1_installed_json() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    composer_vendor(&root, PACKAGES);

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert!(!relative(&root, &found).contains(&"vendor/acme/lib/tests/T.php".to_string()));
    assert!(relative(&root, &found).contains(&"vendor/acme/lib/src/A.php".to_string()));
}

#[test]
fn vendor_without_installed_json_is_indexed_fully() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "vendor/x/y/tests/A.php", "<?php\n");

    let found = discover(&root, &DiscoverOptions::default()).unwrap();

    assert_eq!(relative(&root, &found), vec!["vendor/x/y/tests/A.php"]);
}

#[test]
fn skips_invalid_exclude_globs() {
    let dir = TempDir::new().unwrap();
    let root = canonical_root(&dir);
    write(&root, "skip/A.php", "<?php\n");
    write(&root, "keep/B.php", "<?php\n");
    let opts = DiscoverOptions {
        exclude: vec!["vendor/{a".to_string(), "skip/**".to_string()],
        ..DiscoverOptions::default()
    };

    let found = discover(&root, &opts).unwrap();

    assert_eq!(relative(&root, &found), vec!["keep/B.php"]);
}
