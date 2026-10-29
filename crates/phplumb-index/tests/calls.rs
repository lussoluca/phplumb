use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use phplumb_index::discover::DiscoverOptions;
use phplumb_index::{CallItem, CallKind, Index};
use tempfile::TempDir;

const SOURCE: &str = "<?php\nclass X { public function __construct() {} }\nfunction helper() {}\nclass A\n{\n    public function a($unknown)\n    {\n        $this->b();\n        $this->b();\n        helper();\n        new X();\n        $unknown->c();\n    }\n\n    public function b() {}\n}\nfunction f() { (new A())->b(); }\n(new A())->b();\n";

fn setup() -> (TempDir, PathBuf, Index) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("A.php"), SOURCE).unwrap();
    let index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    (dir, root, index)
}

fn item(index: &mut Index, root: &Path, needle: &str, shift: usize) -> CallItem {
    let path = root.join("A.php");
    let offset = SOURCE.find(needle).unwrap() + shift;
    let mut items = index.call_items(&path, SOURCE.as_bytes(), offset as u32);
    assert_eq!(items.len(), 1, "{items:?}");
    items.remove(0)
}

fn by_name(calls: Vec<(CallItem, Vec<(u32, u32)>)>) -> BTreeMap<String, (CallKind, usize)> {
    calls
        .into_iter()
        .map(|(item, ranges)| (item.name, (item.kind, ranges.len())))
        .collect()
}

#[test]
fn call_items_on_a_call_site() {
    let (_dir, root, mut index) = setup();

    let b = item(&mut index, &root, "b();\n        $this->b();", 0);

    assert_eq!(
        (b.name.as_str(), b.kind, b.detail.as_str()),
        ("b", CallKind::Method, "A")
    );
}

#[test]
fn incoming_calls_group_by_caller() {
    let (_dir, root, mut index) = setup();
    let b = item(&mut index, &root, "function b()", "function ".len());

    let callers = by_name(index.incoming_calls(&b));

    assert_eq!(callers.get("a"), Some(&(CallKind::Method, 2)));
    assert_eq!(callers.get("f"), Some(&(CallKind::Function, 1)));
}

#[test]
fn incoming_calls_include_top_level_code() {
    let (_dir, root, mut index) = setup();
    let b = item(&mut index, &root, "function b()", "function ".len());

    let callers = by_name(index.incoming_calls(&b));

    assert_eq!(callers.get("A.php"), Some(&(CallKind::File, 1)));
}

#[test]
fn outgoing_calls_list_resolved_callees() {
    let (_dir, root, mut index) = setup();
    let a = item(&mut index, &root, "function a(", "function ".len());

    let callees = by_name(index.outgoing_calls(&a));

    assert_eq!(callees.get("b"), Some(&(CallKind::Method, 2)));
    assert_eq!(callees.get("helper"), Some(&(CallKind::Function, 1)));
    assert_eq!(callees.get("__construct"), Some(&(CallKind::Method, 1)));
    assert_eq!(callees.len(), 3, "{callees:?}");
}

fn workspace(source: &str) -> (TempDir, PathBuf, Index) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("A.php"), source).unwrap();
    let index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    (dir, root, index)
}

fn item_in(index: &mut Index, root: &Path, source: &str, needle: &str) -> CallItem {
    let offset = source.find(needle).unwrap();
    let mut items = index.call_items(&root.join("A.php"), source.as_bytes(), offset as u32);
    assert_eq!(items.len(), 1, "{items:?}");
    items.remove(0)
}

#[test]
fn incoming_calls_of_a_constructor_come_from_new() {
    let source = "<?php\nclass X { public function __construct() {} public static function create() { return new static(); } }\nclass Y extends X {}\nclass Z extends X { public function __construct() { parent::__construct(); } }\nfunction make() { new X(); new Y; new Z(); }\n";
    let (_dir, root, mut index) = workspace(source);
    let x = item_in(&mut index, &root, source, "__construct() {} public static");

    let callers = by_name(index.incoming_calls(&x));

    assert_eq!(callers.get("make"), Some(&(CallKind::Function, 2)));
    assert_eq!(callers.get("create"), Some(&(CallKind::Method, 1)));
    assert_eq!(callers.get("__construct"), Some(&(CallKind::Method, 1)));
    assert_eq!(callers.len(), 3, "{callers:?}");
}

#[test]
fn stale_items_give_no_calls() {
    let source =
        "<?php\nfunction a() { helper(); }\nfunction helper() { a(); }\nfunction other() {}\n";
    let (_dir, root, mut index) = workspace(source);
    let a = item_in(&mut index, &root, source, "a() {");
    let path = root.join("A.php");
    let edited = source.replace(
        "<?php\n",
        "<?php\nfunction z() { other(); other(); other(); }\n",
    );
    fs::write(&path, &edited).unwrap();
    index.update(std::slice::from_ref(&path), &[]);

    assert_eq!(index.outgoing_calls(&a), Vec::new());
    assert_eq!(index.incoming_calls(&a), Vec::new());
}

#[test]
fn incoming_calls_label_calls_through_an_ancestor() {
    let source = "<?php\ninterface I { public function save(); }\nclass E implements I { public function save() {} }\nfunction direct(E $e) { $e->save(); }\nfunction through(I $i) { $i->save(); }\n";
    let (_dir, root, mut index) = workspace(source);
    let save = item_in(&mut index, &root, source, "save() {}");

    let details: BTreeMap<String, String> = index
        .incoming_calls(&save)
        .into_iter()
        .map(|(item, _)| (item.name, item.detail))
        .collect();

    assert_eq!(details.get("direct").map(String::as_str), Some(""));
    assert_eq!(
        details.get("through").map(String::as_str),
        Some("via I::save")
    );
}

#[test]
fn outgoing_calls_include_new_static() {
    let source = "<?php\nclass X { public function __construct() {} public static function create() { return new static(); } }\n";
    let (_dir, root, mut index) = workspace(source);
    let create = item_in(&mut index, &root, source, "create(");

    let callees = by_name(index.outgoing_calls(&create));

    assert_eq!(callees.get("__construct"), Some(&(CallKind::Method, 1)));
}

fn found_calls(
    content: &str,
    names: &[&str],
) -> (TempDir, Vec<phplumb_index::frameworks::FoundCall>) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("Calls.php"), format!("<?php\n{content}")).unwrap();
    let index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let project = phplumb_index::frameworks::Project {
        dir: root.clone(),
        excluded: Vec::new(),
    };
    let found = index.calls_named(names, &project);
    (dir, found)
}

#[test]
fn calls_named_lists_resolved_calls_with_literal_arguments() {
    use phplumb_index::frameworks::CallTarget;
    let (_dir, found) = found_calls(
        "interface H { function invokeAll($hook); function alter($type, &$data); function other(); }
class S {
    public function __construct(private H $h) {}
    public function run($x) {
        $this->h->invokeAll('cron');
        $this->h->alter(['a', 'b' . $x], $data);
        $this->h->other();
    }
}
",
        &["invokeAll", "alter"],
    );

    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        found[0].targets,
        vec![CallTarget::Method {
            class: "H".to_string(),
            name: "invokeAll".to_string()
        }]
    );
    assert_eq!(found[0].strings, vec![Some("cron".to_string())]);
    assert_eq!(found[0].lists, vec![Vec::<String>::new()]);
    assert_eq!(found[0].caller.as_deref(), Some("S::run"));
    assert_eq!(
        found[1].targets,
        vec![CallTarget::Method {
            class: "H".to_string(),
            name: "alter".to_string()
        }]
    );
    assert_eq!(found[1].strings, vec![None, None]);
    assert_eq!(found[1].lists, vec![vec!["a".to_string()], Vec::new()]);
    let text = fs::read_to_string(&found[1].location.path).unwrap();
    assert_eq!(
        &text[found[1].location.start as usize..found[1].location.end as usize],
        "alter"
    );
}

#[test]
fn calls_on_receivers_of_unknown_type_have_no_targets() {
    let (_dir, found) = found_calls(
        "function f($x) { $x->invokeAll('cron'); }\n",
        &["invokeAll"],
    );

    assert_eq!(found.len(), 1);
    assert!(found[0].targets.is_empty());
}

#[test]
fn calls_in_functions_name_the_function() {
    let (_dir, found) = found_calls(
        "namespace App;\ninterface H { function invokeAll($hook); }\nfunction f(H $h) { $h->invokeAll('x'); }\n",
        &["invokeAll"],
    );

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].caller.as_deref(), Some("App\\f"));
}
