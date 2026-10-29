use std::path::{Path, PathBuf};

use phplumb_index::Index;
use phplumb_index::discover::DiscoverOptions;
use phplumb_index::frameworks::symfony::Symfony;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/symfony")
        .canonicalize()
        .unwrap()
}

fn build() -> Index {
    Index::build(&fixture(), &DiscoverOptions::default())
        .unwrap()
        .0
}

/// The targets of the route `name`, each as its path relative to the fixture and
/// the text at its range.
fn routes(index: &Index, name: &str) -> Vec<(String, String)> {
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    symfony
        .route_targets(name)
        .iter()
        .map(|target| {
            let location = &target.location;
            let text = std::fs::read(&location.path).unwrap();
            let range = &text[location.start as usize..location.end as usize];
            (
                location
                    .path
                    .strip_prefix(fixture())
                    .unwrap()
                    .display()
                    .to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect()
}

fn pair(path: &str, text: &str) -> (String, String) {
    (path.to_string(), text.to_string())
}

#[test]
fn named_routes_lead_to_their_method() {
    let index = build();
    assert_eq!(
        routes(&index, "home"),
        [pair("src/Controller/HomeController.php", "index")]
    );
}

#[test]
fn class_names_prefix_method_routes() {
    let index = build();
    assert_eq!(
        routes(&index, "admin_users"),
        [pair("src/Controller/AdminController.php", "users")]
    );
}

#[test]
fn default_names_follow_symfony() {
    let index = build();
    let list = [pair("src/Controller/BlogController.php", "listAction")];
    assert_eq!(routes(&index, "app_blog_list"), list);
    assert_eq!(routes(&index, "app_blog_list_1"), list);
    assert_eq!(
        routes(&index, "app_blog_show"),
        [pair("src/Controller/BlogController.php", "show")]
    );
}

#[test]
fn single_route_methods_answer_to_their_fqcn() {
    let index = build();
    assert_eq!(
        routes(&index, "App\\Controller\\BlogController::show"),
        [pair("src/Controller/BlogController.php", "show")]
    );
    assert!(routes(&index, "App\\Controller\\BlogController::listAction").is_empty());
}

#[test]
fn invokable_classes_route_their_invoke() {
    let index = build();
    let invoke = [pair("src/Controller/AboutController.php", "__invoke")];
    assert_eq!(routes(&index, "about"), invoke);
    assert_eq!(routes(&index, "App\\Controller\\AboutController"), invoke);
    assert_eq!(
        routes(&index, "App\\Controller\\AboutController::__invoke"),
        invoke
    );
}

#[test]
fn yaml_routes_lead_to_their_key_and_controller() {
    let index = build();
    assert_eq!(
        routes(&index, "legacy_page"),
        [
            pair("config/routes.yaml", "legacy_page"),
            pair("src/Controller/LegacyController.php", "show"),
        ]
    );
    assert_eq!(
        routes(&index, "dev_tools"),
        [
            pair("config/routes/dev.yaml", "dev_tools"),
            pair("src/Controller/LegacyController.php", "__invoke"),
        ]
    );
    assert!(routes(&index, "controllers").is_empty());
}

#[test]
fn vendor_attribute_routes_give_nothing() {
    let index = build();
    assert!(routes(&index, "vendor_page").is_empty());
}

/// Go to definition inside the `nth` occurrence of `needle` in `file`, one byte in.
fn definition(index: &mut Index, file: &str, needle: &str) -> Vec<(String, String)> {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source)
        .find(needle)
        .unwrap_or_else(|| panic!("no {needle} in {file}")) as u32
        + 1;
    index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| {
            let text = std::fs::read(&location.path).unwrap();
            let range = &text[location.start as usize..location.end as usize];
            (
                location
                    .path
                    .strip_prefix(fixture())
                    .unwrap()
                    .display()
                    .to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect()
}

const LINKS: &str = "src/Controller/LinksController.php";

#[test]
fn route_names_in_php_lead_to_their_routes() {
    let mut index = build();
    assert_eq!(
        definition(&mut index, LINKS, "'home'"),
        [pair("src/Controller/HomeController.php", "index")]
    );
    assert_eq!(
        definition(&mut index, LINKS, "'admin_users'"),
        [pair("src/Controller/AdminController.php", "users")]
    );
    assert_eq!(
        definition(&mut index, LINKS, "'app_blog_show'"),
        [pair("src/Controller/BlogController.php", "show")]
    );
}

/// The definitions of the literal `needle` that follows `context` in `file`.
fn definition_after(
    index: &mut Index,
    file: &str,
    context: &str,
    needle: &str,
) -> Vec<(String, String)> {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&source);
    let start = text
        .find(context)
        .unwrap_or_else(|| panic!("no {context} in {file}"));
    let offset = start + text[start..].find(needle).unwrap() + 1;
    index
        .definition(&path, &source, offset as u32)
        .into_iter()
        .map(|location| {
            let source = std::fs::read(&location.path).unwrap();
            let range = &source[location.start as usize..location.end as usize];
            (
                location
                    .path
                    .strip_prefix(fixture())
                    .unwrap()
                    .display()
                    .to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect()
}

#[test]
fn named_route_arguments_lead_to_their_routes() {
    let mut index = build();
    assert_eq!(
        definition_after(&mut index, LINKS, "route: 'home'", "'home'"),
        [pair("src/Controller/HomeController.php", "index")]
    );
    assert_eq!(
        definition_after(&mut index, LINKS, "name: 'about'", "'about'"),
        [pair("src/Controller/AboutController.php", "__invoke")]
    );
    assert!(definition_after(&mut index, LINKS, "status: 'about'", "'about'").is_empty());
}

#[test]
fn unknown_and_dynamic_route_names_give_nothing() {
    let mut index = build();
    assert!(definition(&mut index, LINKS, "'missing'").is_empty());
    assert!(definition(&mut index, LINKS, "$name);").is_empty());
}

const TWIG: &str = "templates/base.html.twig";

#[test]
fn route_names_in_twig_lead_to_their_routes() {
    let mut index = build();
    assert_eq!(
        definition(&mut index, TWIG, "'home'"),
        [pair("src/Controller/HomeController.php", "index")]
    );
    assert_eq!(
        definition(&mut index, TWIG, "'admin_users'"),
        [pair("src/Controller/AdminController.php", "users")]
    );
    assert_eq!(
        definition(&mut index, TWIG, "'app_blog_show'"),
        [pair("src/Controller/BlogController.php", "show")]
    );
}

#[test]
fn twig_without_a_literal_route_gives_nothing() {
    let mut index = build();
    assert!(definition(&mut index, TWIG, "name)").is_empty());
    assert!(definition(&mut index, TWIG, "\"home\"").is_empty());
    assert!(definition(&mut index, TWIG, "'home' ~").is_empty());
    assert!(definition(&mut index, TWIG, "'home'|").is_empty());
}

#[test]
fn twig_files_get_no_php_answers() {
    let mut index = build();
    let path = fixture().join("templates/php.html.twig");
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&source).into_owned();
    let at = |needle: &str| text.find(needle).unwrap() as u32 + 1;

    assert!(index.hover(&path, &source, at("$crud->")).is_none());
    assert!(
        index
            .definition(&path, &source, at("CrudController"))
            .is_empty()
    );
    assert!(
        index
            .references(&path, &source, at("CrudController"), true)
            .is_empty()
    );
    assert!(
        index
            .implementations(&path, &source, at("generate"))
            .is_empty()
    );
    assert!(index.diagnostics(&path, &source).is_empty());
    assert_eq!(
        definition(&mut index, "templates/php.html.twig", "'home'"),
        [pair("src/Controller/HomeController.php", "index")]
    );
}

/// A copy of the fixture, built.
fn copied() -> (tempfile::TempDir, PathBuf, Index) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&fixture(), &root);
    let index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    (dir, root, index)
}

/// The targets of the route `name`, each as its file name and the text at its range.
fn route_files(index: &Index, name: &str) -> Vec<(String, String)> {
    index
        .framework::<Symfony>()
        .unwrap()
        .route_targets(name)
        .iter()
        .map(|target| {
            let file = target.location.path.file_name().unwrap();
            (file.to_string_lossy().into_owned(), target.text.clone())
        })
        .collect()
}

#[test]
fn route_edits_update_route_targets() {
    let (_dir, root, mut index) = copied();
    let controller = root.join("src/Controller/HomeController.php");
    let text = std::fs::read_to_string(&controller).unwrap();
    std::fs::write(&controller, text.replace("'home'", "'start'")).unwrap();

    index.update(&[controller], &[]);

    assert!(route_files(&index, "home").is_empty());
    assert_eq!(
        route_files(&index, "start"),
        [pair("HomeController.php", "index")]
    );
}

#[test]
fn removed_controllers_lose_their_routes() {
    let (_dir, root, mut index) = copied();
    let controller = root.join("src/Controller/AdminController.php");
    std::fs::remove_file(&controller).unwrap();

    index.update(&[], &[controller]);

    assert!(route_files(&index, "admin_users").is_empty());
}

#[test]
fn route_file_edits_update_route_targets() {
    let (_dir, root, mut index) = copied();
    let routes = root.join("config/routes.yaml");
    let text = std::fs::read_to_string(&routes).unwrap();
    std::fs::write(&routes, text.replace("legacy_page:", "old_page:")).unwrap();

    index.update(&[routes], &[]);

    assert!(route_files(&index, "legacy_page").is_empty());
    assert_eq!(
        route_files(&index, "old_page")[0],
        pair("routes.yaml", "old_page")
    );
}

#[test]
fn added_xml_route_files_make_routes_uncertain() {
    let (_dir, root, mut index) = copied();
    let missing = |index: &Index| index.framework::<Symfony>().unwrap().route_missing("nope");
    assert!(missing(&index));
    let xml = root.join("config/routes/legacy.xml");
    std::fs::write(&xml, "<routes/>\n").unwrap();

    index.update(&[xml], &[]);

    assert!(!missing(&index));
}

#[test]
fn abstract_controllers_define_no_routes() {
    let index = build();
    assert!(routes(&index, "index").is_empty());
    assert!(routes(&index, "app_crud_list").is_empty());
}

#[test]
fn inherited_route_methods_belong_to_the_subclass() {
    let index = build();
    let crud = "src/Controller/CrudController.php";
    assert_eq!(routes(&index, "post_index"), [pair(crud, "index")]);
    assert_eq!(routes(&index, "post_app_post_list"), [pair(crud, "list")]);
    assert_eq!(
        routes(&index, "App\\Controller\\PostController::index"),
        [pair(crud, "index")]
    );
}

/// The file the template `name` resolves to, relative to the fixture.
fn template(index: &Index, name: &str) -> Option<String> {
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    symfony
        .template_file(name)
        .map(|path| path.strip_prefix(fixture()).unwrap().display().to_string())
}

#[test]
fn namespaced_names_use_their_paths() {
    let index = build();
    assert_eq!(
        template(&index, "@Blog/list.html.twig").as_deref(),
        Some("src/Blog/templates/list.html.twig")
    );
    assert_eq!(
        template(&index, "@Blog/card.html.twig").as_deref(),
        Some("src/Shared/templates/card.html.twig")
    );
    assert_eq!(
        template(&index, "@Dev/tools.html.twig").as_deref(),
        Some("dev/templates/tools.html.twig")
    );
}

#[test]
fn plain_names_use_the_main_namespace_then_the_default_path() {
    let index = build();
    assert_eq!(
        template(&index, "base.html.twig").as_deref(),
        Some("extra/base.html.twig")
    );
    assert_eq!(
        template(&index, "only_extra.html.twig").as_deref(),
        Some("extra/only_extra.html.twig")
    );
}

#[test]
fn unknown_names_and_namespaces_give_nothing() {
    let index = build();
    assert_eq!(template(&index, "@Twig/Exception/error.html.twig"), None);
    assert_eq!(template(&index, "missing.html.twig"), None);
}

const PAGES: &str = "src/Controller/PagesController.php";

#[test]
fn template_names_in_php_open_their_file() {
    let mut index = build();
    for (needle, file) in [
        (
            "'@Blog/list.html.twig'",
            "src/Blog/templates/list.html.twig",
        ),
        ("'base.html.twig'", "extra/base.html.twig"),
        (
            "'@Blog/card.html.twig'",
            "src/Shared/templates/card.html.twig",
        ),
        ("'@Dev/tools.html.twig'", "dev/templates/tools.html.twig"),
    ] {
        assert_eq!(
            definition(&mut index, PAGES, needle),
            [pair(file, "")],
            "{needle}"
        );
    }
}

#[test]
fn unknown_template_names_give_nothing() {
    let mut index = build();
    assert!(definition(&mut index, PAGES, "'missing.html.twig'").is_empty());
    // The method name is not the attribute: it never opens the template.
    let on_name = definition(&mut index, PAGES, "tools()");
    assert!(
        on_name.iter().all(|(path, _)| !path.ends_with(".twig")),
        "{on_name:?}"
    );
}

const PAGE: &str = "templates/page.html.twig";

/// Go to definition one byte inside `needle` on line `line` (from 1) of `file`.
fn definition_on_line(
    index: &mut Index,
    file: &str,
    line: usize,
    needle: &str,
) -> Vec<(String, String)> {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&source).into_owned();
    let line_start: usize = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let column = text[line_start..].find(needle).unwrap();
    let offset = (line_start + column + 1) as u32;
    index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| {
            let content = std::fs::read(&location.path).unwrap();
            let range = &content[location.start as usize..location.end as usize];
            (
                location
                    .path
                    .strip_prefix(fixture())
                    .unwrap()
                    .display()
                    .to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect()
}

#[test]
fn template_names_in_twig_open_their_file() {
    let mut index = build();
    for (line, needle, file) in [
        (1, "'base.html.twig'", "extra/base.html.twig"),
        (
            2,
            "'@Blog/list.html.twig'",
            "src/Blog/templates/list.html.twig",
        ),
        (
            3,
            "\"@Blog/card.html.twig\"",
            "src/Shared/templates/card.html.twig",
        ),
        (4, "'only_extra.html.twig'", "extra/only_extra.html.twig"),
        (5, "'@Dev/tools.html.twig'", "dev/templates/tools.html.twig"),
        (6, "'base.html.twig'", "extra/base.html.twig"),
        (
            7,
            "'@Blog/list.html.twig'",
            "src/Blog/templates/list.html.twig",
        ),
        (8, "'base.html.twig'", "extra/base.html.twig"),
        (9, "'base.html.twig'", "extra/base.html.twig"),
    ] {
        assert_eq!(
            definition_on_line(&mut index, PAGE, line, needle),
            [pair(file, "")],
            "line {line}"
        );
    }
}

#[test]
fn twig_template_variables_give_nothing() {
    let mut index = build();
    assert!(definition_on_line(&mut index, PAGE, 10, "template").is_empty());
    assert!(definition_on_line(&mut index, PAGE, 12, "'base.html.twig'").is_empty());
}

#[test]
fn routes_and_templates_on_one_line() {
    let mut index = build();
    assert_eq!(
        definition_on_line(&mut index, PAGE, 11, "'home'"),
        [pair("src/Controller/HomeController.php", "index")]
    );
    assert_eq!(
        definition_on_line(&mut index, PAGE, 11, "'@Blog/card.html.twig'"),
        [pair("src/Shared/templates/card.html.twig", "")]
    );
}

/// Each location as (path relative to the fixture, text at its range), sorted.
fn located(locations: &[phplumb_index::Location]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = locations
        .iter()
        .map(|location| {
            let text = std::fs::read(&location.path).unwrap();
            let range = &text[location.start as usize..location.end as usize];
            (
                location
                    .path
                    .strip_prefix(fixture())
                    .unwrap()
                    .display()
                    .to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect();
    found.sort();
    found
}

fn handlers(index: &Index, message: &str) -> Vec<(String, String)> {
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    located(&symfony.handlers_of(index, message))
}

#[test]
fn handlers_of_a_message() {
    let index = build();
    assert_eq!(
        handlers(&index, "App\\Message\\SendEmail"),
        [
            pair("src/Handler/NotifyLogger.php", "handle"),
            pair("src/Handler/SendEmailHandler.php", "__invoke"),
            pair("src/Handler/SmsHandlers.php", "onSms"),
            pair("src/Handler/StaleDoc.php", "__invoke"),
        ]
    );
    assert_eq!(
        handlers(&index, "App\\Message\\SendSms"),
        [
            pair("src/Handler/Explicit.php", "run"),
            pair("src/Handler/NotifyLogger.php", "handle"),
            pair("src/Handler/SmsHandlers.php", "onSms"),
        ]
    );
}

#[test]
fn messages_without_handlers_give_nothing() {
    let index = build();
    assert!(handlers(&index, "App\\Message\\Unhandled").is_empty());
}

#[test]
fn handlers_name_what_they_handle() {
    let index = build();
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    let handled = |file: &str, method: &str| -> Vec<String> {
        let path = fixture().join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let start = text.find(&format!("function {method}(")).unwrap() + "function ".len();
        let mut found = symfony.handled_by(&path, start as u32);
        found.sort();
        found
    };
    assert_eq!(
        handled("src/Handler/NotifyLogger.php", "handle"),
        ["App\\Message\\Notify"]
    );
    assert_eq!(
        handled("src/Handler/SmsHandlers.php", "onSms"),
        ["App\\Message\\SendEmail", "App\\Message\\SendSms"]
    );
    assert!(handled("src/Handler/NotifyLogger.php", "__invoke").is_empty());
}

const MESSAGES: &str = "src/Controller/MessagesController.php";

/// Go to definition on `dispatch` of line `line` of the messages controller, sorted.
fn dispatch_targets(index: &mut Index, line: usize) -> Vec<(String, String)> {
    let mut found = definition_on_line(index, MESSAGES, line, "dispatch");
    found.sort();
    found
}

#[test]
fn dispatch_lists_the_handlers() {
    let mut index = build();
    assert_eq!(
        dispatch_targets(&mut index, 19),
        [
            pair("src/Handler/NotifyLogger.php", "handle"),
            pair("src/Handler/SendEmailHandler.php", "__invoke"),
            pair("src/Handler/SmsHandlers.php", "onSms"),
            pair("src/Handler/StaleDoc.php", "__invoke"),
        ]
    );
    assert_eq!(
        dispatch_targets(&mut index, 20),
        [
            pair("src/Handler/Explicit.php", "run"),
            pair("src/Handler/NotifyLogger.php", "handle"),
            pair("src/Handler/SmsHandlers.php", "onSms"),
        ]
    );
}

#[test]
fn dispatch_without_handlers_keeps_its_declaration() {
    let mut index = build();
    let declaration = [pair(
        "vendor/symfony/messenger/MessageBusInterface.php",
        "dispatch",
    )];
    for line in [21, 22, 23] {
        assert_eq!(
            dispatch_targets(&mut index, line),
            declaration,
            "line {line}"
        );
    }
}

/// Go to definition at `needle` on `line` of `file`, as (line, text) pairs sorted.
fn lines_of(index: &mut Index, file: &str, line: usize, needle: &str) -> Vec<(usize, String)> {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&source).into_owned();
    let line_start: usize = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let offset = (line_start + text[line_start..].find(needle).unwrap() + 1) as u32;
    let mut found: Vec<(usize, String)> = index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| {
            let content = String::from_utf8(std::fs::read(&location.path).unwrap()).unwrap();
            let at = content[..location.start as usize].matches('\n').count() + 1;
            (
                at,
                content[location.start as usize..location.end as usize].to_string(),
            )
        })
        .collect();
    found.sort();
    found
}

#[test]
fn handlers_list_their_dispatches() {
    let mut index = build();
    let dispatch = |line: usize| (line, "dispatch".to_string());
    assert_eq!(
        lines_of(
            &mut index,
            "src/Handler/SendEmailHandler.php",
            11,
            "__invoke"
        ),
        [dispatch(19)]
    );
    assert_eq!(
        lines_of(&mut index, "src/Handler/NotifyLogger.php", 15, "handle"),
        [dispatch(19), dispatch(20)]
    );
    assert_eq!(
        lines_of(&mut index, "src/Handler/Explicit.php", 11, "run"),
        [dispatch(20)]
    );
}

#[test]
fn the_handler_attribute_lists_the_dispatches() {
    let mut index = build();
    // Inside the attribute's arguments: its name opens the attribute class.
    assert_eq!(
        lines_of(&mut index, "src/Handler/NotifyLogger.php", 8, "method:"),
        [(19, "dispatch".to_string()), (20, "dispatch".to_string())]
    );
}

#[test]
fn dispatch_keeps_hover_and_references() {
    let mut index = build();
    let path = fixture().join(MESSAGES);
    let source = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&source).into_owned();
    let offset = text.find("dispatch(new SendEmail").unwrap() as u32 + 1;
    let hover = index.hover(&path, &source, offset).expect("a hover");
    assert!(
        hover.markdown.contains("dispatch(object $message"),
        "{}",
        hover.markdown
    );
    let references = index.references(&path, &source, offset, false);
    assert_eq!(references.len(), 5, "{references:?}");
}

#[test]
fn handler_targets_follow_edits_on_disk() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&fixture(), &root);
    let mut index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let handler = root.join("src/Handler/SendEmailHandler.php");
    let content = std::fs::read_to_string(&handler).unwrap();
    let shifted = format!("<?php\n// one\n// two\n// three\n{}", &content[6..]);
    std::fs::write(&handler, shifted).unwrap();
    let path = root.join(MESSAGES);
    let source = std::fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source)
        .find("dispatch(new SendEmail")
        .unwrap() as u32
        + 1;
    for location in index.definition(&path, &source, offset) {
        let text = std::fs::read(&location.path).unwrap();
        let name = String::from_utf8_lossy(&text[location.start as usize..location.end as usize])
            .into_owned();
        if location.path == handler {
            assert_eq!(name, "__invoke", "{location:?}");
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn parameters(index: &Index, name: &str) -> Vec<(String, String)> {
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    let mut found: Vec<(String, String)> = symfony
        .parameter_targets(name)
        .iter()
        .map(|target| {
            let text = std::fs::read(&target.location.path).unwrap();
            let range = &text[target.location.start as usize..target.location.end as usize];
            (
                target
                    .location
                    .path
                    .strip_prefix(fixture())
                    .unwrap()
                    .display()
                    .to_string(),
                String::from_utf8_lossy(range).into_owned(),
            )
        })
        .collect();
    found.sort();
    found
}

#[test]
fn parameters_lead_to_their_definitions() {
    let index = build();
    assert_eq!(
        parameters(&index, "app.name"),
        [
            pair("config/packages/framework.yaml", "app.name"),
            pair("config/services.yaml", "app.name"),
        ]
    );
    assert_eq!(
        parameters(&index, "app.limit"),
        [pair("config/services.yaml", "app.limit")]
    );
    assert!(parameters(&index, "env(APP_X)").is_empty());
    assert!(parameters(&index, "kernel.project_dir").is_empty());
}

#[test]
fn parameters_in_yaml_values() {
    let mut index = build();
    let framework = "config/packages/framework.yaml";
    let mut found = definition(&mut index, framework, "%app.name%");
    found.sort();
    assert_eq!(
        found,
        [
            pair("config/packages/framework.yaml", "app.name"),
            pair("config/services.yaml", "app.name"),
        ]
    );
    assert!(definition(&mut index, framework, "%env(APP_X)%").is_empty());
    assert!(definition(&mut index, framework, "%%'").is_empty());
}

#[test]
fn get_parameter_leads_to_the_definition() {
    let mut index = build();
    let params = "src/Controller/ParamsController.php";
    assert_eq!(
        definition(&mut index, params, "'app.limit'"),
        [pair("config/services.yaml", "app.limit")]
    );
    assert!(definition(&mut index, params, "'kernel.project_dir'").is_empty());
}

const CONFIGURED: &str = "src/Service/Configured.php";

#[test]
fn autowire_parameters_lead_to_their_definitions() {
    let mut index = build();
    let mut name = definition(&mut index, CONFIGURED, "app.name");
    name.sort();
    assert_eq!(
        name,
        [
            pair("config/packages/framework.yaml", "app.name"),
            pair("config/services.yaml", "app.name"),
        ]
    );
    assert_eq!(
        definition(&mut index, CONFIGURED, "app.limit"),
        [pair("config/services.yaml", "app.limit")]
    );
    assert!(definition(&mut index, CONFIGURED, "kernel.project_dir").is_empty());
    assert!(definition(&mut index, CONFIGURED, "APP_X").is_empty());
}

const SERVICES: &str = "config/services.yaml";

#[test]
fn yaml_class_names_open_their_class() {
    let mut index = build();
    let mailer = [pair("src/Service/Mailer.php", "Mailer")];
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 14, "App\\Service\\Mailer"),
        mailer,
        "key"
    );
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 19, "App\\Service\\Mailer"),
        mailer,
        "class:"
    );
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 20, "App\\Service\\Mailer"),
        mailer,
        "!service"
    );
    assert_eq!(
        definition_on_line(
            &mut index,
            "config/packages/messenger.yaml",
            4,
            "App\\Message"
        ),
        [pair("src/Message/SendEmail.php", "SendEmail")]
    );
}

#[test]
fn yaml_service_references_open_their_definition() {
    let mut index = build();
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 20, "app.mailer"),
        [pair(SERVICES, "app.mailer")]
    );
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 15, "App\\Service"),
        [pair(SERVICES, "App\\Service\\Mailer")]
    );
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 16, "app.mailer"),
        [pair(SERVICES, "app.mailer")]
    );
    assert!(definition_on_line(&mut index, SERVICES, 17, "service(").is_empty());
}

#[test]
fn autowire_service_opens_its_definition() {
    let mut index = build();
    assert_eq!(
        definition(&mut index, CONFIGURED, "'app.mailer'"),
        [pair(SERVICES, "app.mailer")]
    );
}

#[test]
fn yaml_comments_give_nothing() {
    let mut index = build();
    assert!(definition_on_line(&mut index, SERVICES, 10, "App\\Service").is_empty());
}

#[test]
fn translations_have_no_parameters() {
    let mut index = build();
    assert!(definition(&mut index, "translations/messages.en.yaml", "app.name").is_empty());
}

#[test]
fn alias_and_decorates_open_their_service() {
    let mut index = build();
    let mailer = [pair(SERVICES, "app.mailer")];
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 22, "app.mailer"),
        mailer,
        "alias"
    );
    assert_eq!(
        definition_on_line(&mut index, SERVICES, 25, "app.mailer"),
        mailer,
        "decorates"
    );
}

#[test]
fn repositories_come_from_xml_and_attributes() {
    let index = build();
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    assert_eq!(
        symfony.repository_of("App\\Entity\\Post"),
        Some("App\\Repository\\PostRepository")
    );
    assert_eq!(
        symfony.repository_of("App\\Entity\\Comment"),
        Some("App\\Repository\\CommentRepository")
    );
    assert_eq!(symfony.repository_of("App\\Entity\\Author"), None);
}

const POST_XML: &str = "config/doctrine/Post.orm.xml";
const AUTHOR_XML: &str = "config/doctrine/Author.orm.xml";

#[test]
fn mapping_class_names_open_their_class() {
    let mut index = build();
    for (line, needle, file, text) in [
        (3, "App\\Entity\\Post", "src/Entity/Post.php", "Post"),
        (
            3,
            "App\\Repository",
            "src/Repository/PostRepository.php",
            "PostRepository",
        ),
        (6, "\"Author\"", "src/Entity/Author.php", "Author"),
        (
            7,
            "App\\Entity\\Address",
            "src/Entity/Address.php",
            "Address",
        ),
    ] {
        assert_eq!(
            definition_on_line(&mut index, POST_XML, line, needle),
            [pair(file, text)],
            "{needle}"
        );
    }
}

#[test]
fn mapping_property_names_open_their_property() {
    let mut index = build();
    let post = "src/Entity/Post.php";
    for (line, needle, text) in [
        (4, "\"id\"", "id"),
        (5, "\"title\"", "title"),
        (6, "\"author\"", "author"),
        (7, "\"address\"", "address"),
    ] {
        assert_eq!(
            definition_on_line(&mut index, POST_XML, line, needle),
            [pair(post, text)],
            "{needle}"
        );
    }
    assert_eq!(
        definition_on_line(&mut index, POST_XML, 6, "\"posts\""),
        [pair("src/Entity/Author.php", "posts")]
    );
    assert_eq!(
        definition_on_line(&mut index, AUTHOR_XML, 6, "\"author\""),
        [pair(post, "author")]
    );
    assert_eq!(
        definition_on_line(&mut index, "config/doctrine/Address.orm.xml", 4, "street"),
        [pair("src/Entity/Address.php", "street")]
    );
}

#[test]
fn mapping_comments_and_namespaces_give_nothing() {
    let mut index = build();
    assert!(definition_on_line(&mut index, AUTHOR_XML, 3, "App\\Entity").is_empty());
    assert!(definition_on_line(&mut index, POST_XML, 2, "doctrine-project").is_empty());
}

const POSTS: &str = "src/Service/Posts.php";

#[test]
fn get_repository_returns_the_mapped_repository() {
    let mut index = build();
    assert_eq!(
        definition_on_line(&mut index, POSTS, 20, "findRecent"),
        [pair("src/Repository/PostRepository.php", "findRecent")]
    );
    assert_eq!(
        definition_on_line(&mut index, POSTS, 21, "findThread"),
        [pair("src/Repository/CommentRepository.php", "findThread")]
    );
}

#[test]
fn get_repository_without_a_mapping_keeps_its_type() {
    let mut index = build();
    let declared = [pair(
        "vendor/doctrine/persistence/src/ObjectRepository.php",
        "find",
    )];
    assert_eq!(definition_on_line(&mut index, POSTS, 22, "find("), declared);
    assert_eq!(definition_on_line(&mut index, POSTS, 23, "find("), declared);
}

#[test]
fn short_enum_types_are_not_qualified() {
    let mut index = build();
    assert!(definition_on_line(&mut index, POST_XML, 8, "Status").is_empty());
}

#[test]
fn string_repository_classes_are_qualified_and_must_exist() {
    let mut index = build();
    assert_eq!(
        definition_on_line(&mut index, POSTS, 24, "findTagged"),
        [pair("src/Entity/TagRepository.php", "findTagged")]
    );
    assert_eq!(
        definition_on_line(&mut index, POSTS, 25, "find("),
        [pair(
            "vendor/doctrine/persistence/src/ObjectRepository.php",
            "find"
        )]
    );
}

#[test]
fn excluded_mappings_are_not_read() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&fixture(), &root);
    std::fs::create_dir_all(root.join("legacy")).unwrap();
    std::fs::write(
        root.join("legacy/Post.orm.xml"),
        "<doctrine-mapping><entity name=\"App\\Entity\\Post\" repository-class=\"App\\Repository\\CommentRepository\"/></doctrine-mapping>\n",
    )
    .unwrap();
    let opts = DiscoverOptions {
        exclude: vec!["legacy/**".to_string()],
        ..DiscoverOptions::default()
    };
    let index = Index::build(&root, &opts).unwrap().0;
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    assert_eq!(
        symfony.repository_of("App\\Entity\\Post"),
        Some("App\\Repository\\PostRepository")
    );
}

const BROKEN: &str = "src/Controller/BrokenController.php";

/// The diagnostics of `file`, each as its message and the text at its range.
fn diagnostics(index: &Index, file: &str) -> Vec<(String, String)> {
    let path = fixture().join(file);
    let source = std::fs::read(&path).unwrap();
    index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| {
            let text = String::from_utf8_lossy(&source[d.start as usize..d.end as usize]);
            (d.message, text.into_owned())
        })
        .collect()
}

#[test]
fn broken_symfony_references_are_flagged() {
    let index = build();
    assert_eq!(
        diagnostics(&index, BROKEN),
        [
            pair("Undefined parameter 'app.missing'", "app.missing"),
            pair("Undefined parameter 'app.gone'", "app.gone"),
            pair("Undefined route 'missing_route'", "missing_route"),
            pair(
                "Undefined template 'nowhere.html.twig'",
                "nowhere.html.twig"
            ),
            pair("Undefined parameter 'app.nowhere'", "app.nowhere"),
            pair("Undefined template 'gone.html.twig'", "gone.html.twig"),
        ]
    );
}

#[test]
fn twig_bridge_form_themes_are_templates() {
    let index = build();
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    assert_eq!(
        symfony.template_file("form_div_layout.html.twig"),
        Some(
            fixture()
                .join("vendor/symfony/twig-bridge/Resources/views/Form/form_div_layout.html.twig")
        )
    );
}

#[test]
fn broken_names_in_twig_are_flagged() {
    let index = build();
    assert_eq!(
        diagnostics(&index, "templates/broken.html.twig"),
        [
            pair("Undefined route 'missing_route'", "missing_route"),
            pair(
                "Undefined template 'nowhere.html.twig'",
                "nowhere.html.twig"
            ),
            pair("Undefined route 'nope_route'", "nope_route"),
            pair("Undefined route 'nope_named'", "nope_named"),
            pair(
                "Undefined template 'gone_block.html.twig'",
                "gone_block.html.twig"
            ),
            pair(
                "Undefined template 'gone_theme.html.twig'",
                "gone_theme.html.twig"
            ),
            pair(
                "Undefined template 'gone_trim.html.twig'",
                "gone_trim.html.twig"
            ),
        ]
    );
    // A list names templates to try in turn.
    assert!(diagnostics(&index, "templates/page.html.twig").is_empty());
}

#[test]
fn broken_parameters_in_yaml_are_flagged() {
    let index = build();
    assert_eq!(
        diagnostics(&index, "config/packages/framework.yaml"),
        [pair("Undefined parameter 'app.nowhere'", "app.nowhere")]
    );
    // Translations use `%name%` placeholders.
    assert!(diagnostics(&index, "translations/messages.en.yaml").is_empty());
}

#[test]
fn self_built_twig_environments_and_generators_stay_silent() {
    let index = build();
    assert!(diagnostics(&index, "tests/Twig/ExtensionTest.php").is_empty());
    assert!(diagnostics(&index, "tests/Routing/GeneratorTest.php").is_empty());
    // Templates outside the configured directories are not the app's.
    assert!(diagnostics(&index, "tests/Fixtures/views/page.html.twig").is_empty());
}

#[test]
fn php_updates_keep_the_yaml_read_before() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&fixture(), &root);
    let mut index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let services = root.join("config/services.yaml");
    let text = std::fs::read_to_string(&services).unwrap();
    std::fs::write(&services, text.replace("app.limit", "app.ceiling")).unwrap();
    let count = |index: &Index, name: &str| {
        index
            .framework::<Symfony>()
            .unwrap()
            .parameter_targets(name)
            .len()
    };

    index.update(&[root.join("src/Controller/HomeController.php")], &[]);
    assert_eq!(count(&index, "app.limit"), 1);

    index.update(&[services], &[]);
    assert_eq!(count(&index, "app.limit"), 0);
    assert_eq!(count(&index, "app.ceiling"), 1);
}

#[cfg(unix)]
#[test]
fn symlinked_config_follows_edits_reported_at_the_real_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&fixture(), &root);
    std::fs::create_dir_all(root.join("shared")).unwrap();
    let shared = root.join("shared/extra.yaml");
    std::fs::write(&shared, "parameters:\n    app.extra: 1\n").unwrap();
    std::os::unix::fs::symlink(&shared, root.join("config/packages/extra.yaml")).unwrap();
    let controller = root.join("src/Controller/ParamsController.php");
    let text = std::fs::read_to_string(&controller).unwrap();
    std::fs::write(&controller, text.replace("app.limit", "app.other")).unwrap();
    let mut index = Index::build(&root, &DiscoverOptions::default()).unwrap().0;
    let count = |index: &Index, name: &str| {
        index
            .framework::<Symfony>()
            .unwrap()
            .parameter_targets(name)
            .len()
    };
    assert_eq!(count(&index, "app.extra"), 1);

    std::fs::write(&shared, "parameters:\n    app.other: 1\n").unwrap();
    index.update(&[shared], &[]);

    let source = std::fs::read(&controller).unwrap();
    let messages: Vec<String> = index
        .diagnostics(&controller, &source)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert_eq!(count(&index, "app.other"), 1);
    assert_eq!(count(&index, "app.extra"), 0);
    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn routes_named_by_constants() {
    let index = build();
    assert_eq!(
        routes(&index, "app_login"),
        [pair("src/Controller/LoginController.php", "login")]
    );
}

#[test]
fn class_routes_named_null_have_no_prefix() {
    let index = build();
    assert_eq!(
        routes(&index, "panel_home"),
        [pair("src/Controller/PanelController.php", "home")]
    );
}

#[test]
fn route_aliases_lead_to_their_route() {
    let index = build();
    assert_eq!(
        routes(&index, "old_aliased"),
        [pair("src/Controller/LocalizedController.php", "aliased")]
    );
    assert_eq!(routes(&index, "legacy_home"), routes(&index, "home"));
    assert!(!routes(&index, "home").is_empty());
}

#[test]
fn localized_routes_have_a_name_per_locale() {
    let index = build();
    for name in ["about_us", "about_us.en", "about_us.it"] {
        assert_eq!(
            routes(&index, name),
            [pair("src/Controller/LocalizedController.php", "about")],
            "{name}"
        );
    }
}

#[test]
fn localized_methods_get_no_class_alias() {
    let index = build();
    assert!(routes(&index, "App\\Controller\\LocalizedController::about").is_empty());
    assert!(!routes(&index, "App\\Controller\\LocalizedController::aliased").is_empty());
}

/// Writes `content` to `file` in the copy at `root` and updates the index.
fn add_file(index: &mut Index, root: &Path, file: &str, content: &str) -> PathBuf {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    index.update(std::slice::from_ref(&path), &[]);
    path
}

/// The files the definitions of the literal in `needle`, at its first quote, in
/// `path` lead to, by name.
fn opened(index: &mut Index, path: &Path, needle: &str) -> Vec<String> {
    let source = std::fs::read(path).unwrap();
    let quote = needle.find(['\'', '"']).unwrap_or(0);
    let offset = (String::from_utf8_lossy(&source).find(needle).unwrap() + quote) as u32 + 1;
    index
        .definition(path, &source, offset)
        .iter()
        .map(|location| {
            let parent = location.path.parent().unwrap().file_name().unwrap();
            let name = location.path.file_name().unwrap();
            format!("{}/{}", parent.to_string_lossy(), name.to_string_lossy())
        })
        .collect()
}

fn messages(index: &Index, path: &Path) -> Vec<String> {
    let source = std::fs::read(path).unwrap();
    index
        .diagnostics(path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn normalized_template_names_open_their_file() {
    let (_dir, root, mut index) = copied();
    add_file(&mut index, &root, "templates/sub/keep.html.twig", "keep\n");
    let path = add_file(
        &mut index,
        &root,
        "templates/normalized.html.twig",
        "{% include 'sub/../page.html.twig' %}\n{% include './/page.html.twig' %}\n{% include '../page.html.twig' %}\n{% include 'nope/../page.html.twig' %}\n",
    );

    assert_eq!(
        opened(&mut index, &path, "'sub/../"),
        ["templates/page.html.twig"]
    );
    assert_eq!(
        opened(&mut index, &path, "'.//"),
        ["templates/page.html.twig"]
    );
    assert!(opened(&mut index, &path, "'../").is_empty());
    // The operating system resolves `..` after a directory that must exist.
    assert!(opened(&mut index, &path, "'nope/").is_empty());
    assert_eq!(
        messages(&index, &path),
        ["Undefined template 'nope/../page.html.twig'"]
    );
}

#[test]
fn templated_emails_open_their_templates() {
    let (_dir, root, mut index) = copied();
    let path = add_file(
        &mut index,
        &root,
        "src/Mail.php",
        "<?php\nnamespace App;\nuse Symfony\\Bridge\\Twig\\Mime\\TemplatedEmail;\nfunction mail(): void\n{\n    $email = new TemplatedEmail();\n    $email->htmlTemplate('page.html.twig');\n    $email->textTemplate('gone_text.html.twig');\n}\n",
    );

    assert_eq!(
        opened(&mut index, &path, "'page"),
        ["templates/page.html.twig"]
    );
    assert_eq!(
        messages(&index, &path),
        ["Undefined template 'gone_text.html.twig'"]
    );
}

#[test]
fn named_view_arguments_open_their_template() {
    let (_dir, root, mut index) = copied();
    let path = add_file(
        &mut index,
        &root,
        "src/Controller/NamedView.php",
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController;\nclass NamedView extends AbstractController\n{\n    public function show(): void\n    {\n        $this->render(view: 'page.html.twig');\n        $this->render(view: 'gone_named.html.twig');\n    }\n}\n",
    );

    assert_eq!(
        opened(&mut index, &path, "'page"),
        ["templates/page.html.twig"]
    );
    assert_eq!(
        messages(&index, &path),
        ["Undefined template 'gone_named.html.twig'"]
    );
}

#[test]
fn class_template_attributes_open_their_template() {
    let (_dir, root, mut index) = copied();
    let path = add_file(
        &mut index,
        &root,
        "src/Controller/ClassTemplate.php",
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Bridge\\Twig\\Attribute\\Template;\n#[Template('page.html.twig')]\nclass ClassTemplate\n{\n    public function __invoke(): array { return []; }\n}\n#[Template('gone_class.html.twig')]\nclass GoneTemplate\n{\n    public function __invoke(): array { return []; }\n}\n",
    );

    assert_eq!(
        opened(&mut index, &path, "'page"),
        ["templates/page.html.twig"]
    );
    assert_eq!(
        messages(&index, &path),
        ["Undefined template 'gone_class.html.twig'"]
    );
}

#[test]
fn render_block_and_stream_open_their_template() {
    let (_dir, root, mut index) = copied();
    let path = add_file(
        &mut index,
        &root,
        "src/Controller/Blocks.php",
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController;\nclass Blocks extends AbstractController\n{\n    public function show(): void\n    {\n        $this->renderBlock('page.html.twig', 'a');\n        $this->renderBlockView('page.html.twig', 'b');\n        $this->stream('page.html.twig');\n    }\n}\n",
    );

    for needle in ["renderBlock('page", "renderBlockView('page", "stream('page"] {
        assert_eq!(
            opened(&mut index, &path, needle),
            ["templates/page.html.twig"],
            "{needle}"
        );
    }
}

#[test]
fn environment_display_and_load_open_their_template() {
    let (_dir, root, mut index) = copied();
    let path = add_file(
        &mut index,
        &root,
        "src/Display.php",
        "<?php\nnamespace App;\nuse Twig\\Environment;\nfunction display(Environment $twig): void\n{\n    $twig->display('page.html.twig');\n    $twig->load('base.html.twig');\n}\n",
    );

    assert_eq!(
        opened(&mut index, &path, "display('page"),
        ["templates/page.html.twig"]
    );
    assert_eq!(
        opened(&mut index, &path, "load('base"),
        ["extra/base.html.twig"]
    );
}

#[test]
fn template_names_in_yaml_open_their_file() {
    let mut index = build();
    let twig = "config/packages/twig.yaml";

    assert_eq!(
        definition(&mut index, twig, "'page.html.twig'"),
        [pair("templates/page.html.twig", "")]
    );
    assert_eq!(
        definition(&mut index, twig, "'@Blog/list.html.twig'"),
        [pair("src/Blog/templates/list.html.twig", "")]
    );
    assert!(definition(&mut index, twig, "'nowhere.html.twig'").is_empty());
    assert!(diagnostics(&index, twig).is_empty());
}
