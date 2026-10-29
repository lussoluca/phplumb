//! Drives the `phplumb` binary over stdio the way an editor does.

mod common;

use common::Client;
use serde_json::json;
use tempfile::TempDir;

fn workspace(files: &[(&str, &str)]) -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (path, content) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let root_uri = format!("file://{}", root.display());
    (dir, root_uri)
}

#[test]
fn definition_over_stdio() {
    let (_dir, root_uri) = workspace(&[
        ("src/Target.php", "<?php\nnamespace App;\nclass Target {}\n"),
        (
            "src/Use.php",
            "<?php\nnamespace App;\n$x = 'è'; new Target();\n",
        ),
    ]);

    let mut client = Client::start();
    client.initialize(json!({
        "processId": null,
        "capabilities": {},
        "workspaceFolders": [{"uri": root_uri, "name": "test"}],
    }));
    let result = client.request(
        2,
        "textDocument/definition",
        json!({
            "textDocument": {"uri": format!("{root_uri}/src/Use.php")},
            "position": {"line": 2, "character": 14},
        }),
    );

    let locations = result.as_array().expect("definition returns an array");
    assert_eq!(locations.len(), 1);
    assert!(
        locations[0]["uri"]
            .as_str()
            .unwrap()
            .ends_with("/src/Target.php")
    );
    assert_eq!(
        locations[0]["range"]["start"],
        json!({"line": 2, "character": 6})
    );
    client.shutdown();
}

#[test]
fn logs_indexing_at_info_level() {
    let (_dir, root_uri) = workspace(&[
        ("src/A.php", "<?php\nclass A {}\n"),
        ("src/B.php", "<?php\nclass B {}\n"),
    ]);

    let mut client = Client::start();
    client.initialize(json!({
        "processId": null,
        "capabilities": {},
        "workspaceFolders": [{"uri": root_uri, "name": "test"}],
    }));
    let log = client.wait_for_notification("window/logMessage");

    assert_eq!(log["params"]["type"], 3);
    assert!(
        log["params"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Indexed 2 files in "),
        "{log}"
    );
    client.shutdown();
}

#[test]
fn hover_over_stdio() {
    let (_dir, root_uri) = workspace(&[
        ("src/Target.php", "<?php\nnamespace App;\nclass Target {}\n"),
        ("src/Use.php", "<?php\nnamespace App;\nnew Target();\n"),
    ]);

    let mut client = Client::start();
    client.initialize(json!({
        "processId": null,
        "capabilities": {},
        "workspaceFolders": [{"uri": root_uri, "name": "test"}],
    }));
    let result = client.request(
        2,
        "textDocument/hover",
        json!({
            "textDocument": {"uri": format!("{root_uri}/src/Use.php")},
            "position": {"line": 2, "character": 5},
        }),
    );

    assert_eq!(result["contents"]["kind"], "markdown");
    assert!(
        result["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("class Target"),
        "{result}"
    );
    client.shutdown();
}

#[test]
fn logs_the_folder_of_each_framework_module() {
    let (_dir, root_uri) = workspace(&[
        ("src/A.php", "<?php\nclass A {}\n"),
        (
            "composer.lock",
            r#"{"packages": [{"name": "symfony/framework-bundle"}]}"#,
        ),
        (
            "codebase/drupal/composer.lock",
            r#"{"packages": [{"name": "drupal/core"}]}"#,
        ),
    ]);

    let mut client = Client::start();
    client.initialize(json!({
        "processId": null,
        "capabilities": {},
        "workspaceFolders": [{"uri": root_uri, "name": "test"}],
    }));
    let log = client.wait_for_notification("window/logMessage");

    let message = log["params"]["message"].as_str().unwrap();
    assert!(
        message.ends_with("; frameworks: drupal (codebase/drupal), symfony)"),
        "{message}"
    );
    client.shutdown();
}
