use std::fs;
use std::path::{Path, PathBuf};

use phplumb_index::Index;
use phplumb_index::discover::DiscoverOptions;
use phplumb_index::frameworks::drupal::Drupal;
use phplumb_index::frameworks::symfony::Symfony;
use phplumb_index::frameworks::{Activation, FrameworkSettings, Project};
use tempfile::TempDir;

const DRUPAL_LOCK: &str = r#"{"packages": [{"name": "drupal/core", "version": "11.4.0"}]}"#;
const SYMFONY_LOCK: &str =
    r#"{"packages": [{"name": "symfony/framework-bundle", "version": "8.1.0"}]}"#;
const DRUPAL_WITH_SYMFONY_PARTS: &str = r#"{"packages": [{"name": "drupal/core", "version": "11.4.0"}, {"name": "symfony/http-kernel", "version": "7.3.0"}]}"#;
const BOTH_LOCK: &str = r#"{"packages": [{"name": "drupal/core", "version": "11.4.0"}], "packages-dev": [{"name": "symfony/framework-bundle", "version": "8.1.0"}]}"#;

/// An empty project with one PHP file, and its canonical root.
fn project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("index.php"), "<?php\n").unwrap();
    (dir, root)
}

fn lock(root: &Path, dir: &str, content: &str) {
    let dir = root.join(dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("composer.lock"), content).unwrap();
}

fn build(root: &Path, settings: FrameworkSettings) -> Index {
    let opts = DiscoverOptions {
        frameworks: settings,
        ..DiscoverOptions::default()
    };
    Index::build(root, &opts).unwrap().0
}

fn auto(root: &Path) -> Index {
    build(root, FrameworkSettings::default())
}

fn scope(index: &Index, name: &str) -> (PathBuf, Vec<PathBuf>) {
    let project = index
        .framework_project(name)
        .unwrap_or_else(|| panic!("{name} is not active"));
    (project.dir.clone(), project.excluded.clone())
}

#[test]
fn symfony_is_detected_from_framework_bundle() {
    let (_dir, root) = project();
    lock(&root, "", SYMFONY_LOCK);
    assert_eq!(auto(&root).active_frameworks(), ["symfony"]);
}

#[test]
fn drupal_projects_never_activate_symfony() {
    let (_dir, root) = project();
    lock(&root, "", DRUPAL_WITH_SYMFONY_PARTS);
    assert_eq!(auto(&root).active_frameworks(), ["drupal"]);
}

#[test]
fn the_nearest_lock_wins() {
    let (_dir, root) = project();
    lock(&root, "a", DRUPAL_LOCK);
    lock(&root, "b/c", DRUPAL_LOCK);
    assert_eq!(scope(&auto(&root), "drupal"), (root.join("a"), vec![]));
}

#[test]
fn locks_at_the_same_depth_take_the_first_in_path_order() {
    let (_dir, root) = project();
    lock(&root, "b", DRUPAL_LOCK);
    lock(&root, "a", DRUPAL_LOCK);
    assert_eq!(scope(&auto(&root), "drupal"), (root.join("a"), vec![]));
}

#[test]
fn nested_projects_are_left_out_of_the_outer_one() {
    let (_dir, root) = project();
    lock(&root, "", SYMFONY_LOCK);
    lock(&root, "codebase/drupal", DRUPAL_LOCK);
    let index = auto(&root);
    assert_eq!(
        scope(&index, "symfony"),
        (root.clone(), vec![root.join("codebase/drupal")])
    );
    assert_eq!(
        scope(&index, "drupal"),
        (root.join("codebase/drupal"), vec![])
    );
}

#[test]
fn one_lock_with_both_packages_activates_both_on_the_root() {
    let (_dir, root) = project();
    lock(&root, "", BOTH_LOCK);
    let index = auto(&root);
    assert_eq!(index.active_frameworks(), ["drupal", "symfony"]);
    assert_eq!(scope(&index, "drupal"), (root.clone(), vec![]));
    assert_eq!(scope(&index, "symfony"), (root.clone(), vec![]));
}

#[test]
fn a_forced_module_without_a_lock_reads_the_root() {
    let (_dir, root) = project();
    let index = build(
        &root,
        FrameworkSettings {
            drupal: Activation::On,
            ..FrameworkSettings::default()
        },
    );
    assert_eq!(scope(&index, "drupal"), (root.clone(), vec![]));
}

#[test]
fn a_forced_module_uses_a_deeper_lock() {
    let (_dir, root) = project();
    lock(&root, "site", DRUPAL_LOCK);
    let index = build(
        &root,
        FrameworkSettings {
            drupal: Activation::On,
            ..FrameworkSettings::default()
        },
    );
    assert_eq!(scope(&index, "drupal"), (root.join("site"), vec![]));
}

#[test]
fn project_contains_compares_whole_components() {
    let project = Project {
        dir: PathBuf::from("/p/drupal"),
        excluded: vec![PathBuf::from("/p/drupal/sub")],
    };
    assert!(project.contains(Path::new("/p/drupal/x.php")));
    assert!(!project.contains(Path::new("/p/drupal2/x.php")));
    assert!(!project.contains(Path::new("/p/drupal/sub/x.php")));
    assert!(project.contains(Path::new("/p/drupal/subway/x.php")));
}

/// The fixture of a Symfony project holding a Drupal codebase.
fn nested() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .canonicalize()
        .unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn drupal(index: &Index) -> &Drupal {
    index.framework::<Drupal>().expect("drupal is active")
}

#[test]
fn drupal_reads_only_its_project() {
    let index = auto(&nested());
    assert!(drupal(&index).defines_route("demo.page"));
    assert!(drupal(&index).defines_route("demo.attr"));
    assert!(!drupal(&index).defines_route("symfony_home"));
}

#[test]
fn updates_outside_the_drupal_project_are_ignored() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&nested(), &root);
    let mut index = auto(&root);
    let late = root.join("src/Controller/LateController.php");
    fs::write(
        &late,
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass LateController\n{\n    #[Route('/late', name: 'symfony_late')]\n    public function late(): void {}\n}\n",
    )
    .unwrap();
    index.update(&[late], &[]);
    assert!(!drupal(&index).defines_route("symfony_late"));
}

const SYMFONY_USES: &str = "src/Uses.php";
const DRUPAL_USES: &str = "codebase/drupal/web/modules/custom/demo/src/Uses.php";

/// Go to definition on `run` in the fixture file `file`.
fn run_targets(index: &mut Index, file: &str) -> Vec<PathBuf> {
    let path = nested().join(file);
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find("run()").unwrap() as u32;
    index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect()
}

fn diagnostics_in(index: &Index, file: &str) -> Vec<String> {
    let path = nested().join(file);
    let source = fs::read(&path).unwrap();
    index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn drupal_types_nothing_outside_its_project() {
    let mut index = auto(&nested());
    let thing = nested().join("codebase/drupal/web/modules/custom/demo/src/Thing.php");
    assert_eq!(run_targets(&mut index, DRUPAL_USES), [thing]);
    assert!(run_targets(&mut index, SYMFONY_USES).is_empty());
}

#[test]
fn drupal_flags_nothing_outside_its_project() {
    let index = auto(&nested());
    let drupal = diagnostics_in(&index, DRUPAL_USES);
    assert_eq!(drupal.len(), 1, "{drupal:?}");
    assert!(drupal[0].contains("missing.service"), "{drupal:?}");
    assert!(diagnostics_in(&index, SYMFONY_USES).is_empty());
}

#[test]
fn updating_a_directory_that_holds_the_drupal_project_keeps_it() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&nested(), &root);
    let mut index = auto(&root);
    for holder in [root.join("codebase"), root.clone()] {
        index.update(std::slice::from_ref(&holder), &[]);
        assert!(drupal(&index).defines_route("demo.page"), "{holder:?}");
        assert!(
            !drupal(&index).services("demo.thing").is_empty(),
            "{holder:?}"
        );
    }
}

#[test]
fn symfony_sees_no_drupal_routes() {
    let mut index = auto(&nested());
    let path = nested().join("src/Controller/LinkController.php");
    let source = fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&source).into_owned();
    let at = |needle: &str| text.find(needle).unwrap() as u32 + 1;
    assert!(
        index
            .definition(&path, &source, at("'demo.attr'"))
            .is_empty()
    );
    let home: Vec<PathBuf> = index
        .definition(&path, &source, at("'symfony_home'"))
        .into_iter()
        .map(|location| location.path)
        .collect();
    assert_eq!(home, [nested().join("src/Controller/HomeController.php")]);
}

#[test]
fn templates_without_paths_use_the_default_directory() {
    let (_dir, root) = project();
    lock(&root, "", SYMFONY_LOCK);
    fs::create_dir_all(root.join("config/packages")).unwrap();
    fs::write(root.join("config/packages/twig.yaml"), "twig: {}\n").unwrap();
    fs::create_dir_all(root.join("templates")).unwrap();
    fs::write(root.join("templates/page.html.twig"), "<p></p>\n").unwrap();
    let index = auto(&root);
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    assert_eq!(
        symfony.template_file("page.html.twig"),
        Some(root.join("templates/page.html.twig"))
    );
}

/// A Symfony project with `twig_yaml` as `config/packages/twig.yaml` and each of
/// `files` as an empty template; the file `name` resolves to, relative to the root.
fn resolve_template(twig_yaml: &str, files: &[&str], name: &str) -> Option<String> {
    let (_dir, root) = project();
    lock(&root, "", SYMFONY_LOCK);
    fs::create_dir_all(root.join("config/packages")).unwrap();
    fs::write(root.join("config/packages/twig.yaml"), twig_yaml).unwrap();
    for file in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "<p></p>\n").unwrap();
    }
    let index = auto(&root);
    let symfony = index.framework::<Symfony>().expect("symfony is active");
    symfony
        .template_file(name)
        .map(|path| path.strip_prefix(&root).unwrap().display().to_string())
}

#[test]
fn environment_blocks_never_shadow_the_main_paths() {
    let yaml = "twig: {}\nwhen@test:\n    twig:\n        default_path: '%kernel.project_dir%/tests/templates'\n        paths:\n            '%kernel.project_dir%/tests/fixtures': ~\n            '%kernel.project_dir%/tests/ns': Fix\n";
    let files = [
        "templates/base.html.twig",
        "tests/templates/base.html.twig",
        "tests/fixtures/base.html.twig",
        "tests/ns/x.html.twig",
    ];
    assert_eq!(
        resolve_template(yaml, &files, "base.html.twig").as_deref(),
        Some("templates/base.html.twig")
    );
    assert_eq!(
        resolve_template(yaml, &files, "@Fix/x.html.twig").as_deref(),
        Some("tests/ns/x.html.twig")
    );
}

#[test]
fn an_unreadable_path_stops_its_namespace() {
    let files = ["templates/base.html.twig", "theme/base.html.twig"];
    assert_eq!(
        resolve_template(
            "twig:\n    paths:\n        '%app.theme_dir%': ~\n",
            &files,
            "base.html.twig"
        ),
        None
    );
    assert_eq!(
        resolve_template(
            "twig:\n    paths:\n        'theme': NULL\n",
            &files,
            "base.html.twig"
        )
        .as_deref(),
        Some("theme/base.html.twig")
    );
}

#[test]
fn drupal_and_symfony_yaml_stay_apart() {
    let mut index = auto(&nested());
    let path = nested().join("config/services.yaml");
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find("demo.thing").unwrap() as u32 + 1;
    assert!(index.definition(&path, &source, offset).is_empty());
    let drupal = nested().join("codebase/drupal/web/modules/custom/demo/demo.services.yml");
    let source = fs::read(&drupal).unwrap();
    let offset = String::from_utf8_lossy(&source).find("Thing").unwrap() as u32 + 1;
    let targets: Vec<PathBuf> = index
        .definition(&drupal, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect();
    assert_eq!(
        targets,
        [nested().join("codebase/drupal/web/modules/custom/demo/src/Thing.php")]
    );
}

#[test]
fn doctrine_xml_stays_in_its_project() {
    let mut index = auto(&nested());
    let path = nested().join("codebase/drupal/web/modules/custom/demo/config/Thing.orm.xml");
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find("Thing\"").unwrap() as u32;
    assert!(index.definition(&path, &source, offset).is_empty());
}

const ABSTRACT_CONTROLLER: &str = "<?php\nnamespace Symfony\\Bundle\\FrameworkBundle\\Controller;\nabstract class AbstractController\n{\n    protected function redirectToRoute(string $route): object { return new \\stdClass(); }\n    protected function getParameter(string $name): mixed { return null; }\n}\n";

const BROKEN_USES: &str = "<?php\nnamespace App\\Controller;\nuse Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController;\nclass Uses extends AbstractController\n{\n    public function run(): void\n    {\n        $this->redirectToRoute('missing_route');\n        $this->getParameter('app.nowhere');\n    }\n}\n";

/// A Symfony project with a controller that names a missing route and a missing
/// parameter, a route and a parameter it defines, and `files`.
fn broken_symfony(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let (dir, root) = project();
    lock(&root, ".", SYMFONY_LOCK);
    let mut all = vec![
        ("lib/AbstractController.php", ABSTRACT_CONTROLLER),
        ("src/Controller/Uses.php", BROKEN_USES),
        ("config/routes.yaml", "home:\n    path: /\n"),
        ("config/services.yaml", "parameters:\n    app.name: x\n"),
    ];
    all.extend_from_slice(files);
    for (file, content) in all {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    (dir, root)
}

fn broken_messages(root: &Path) -> Vec<String> {
    let index = auto(root);
    let path = root.join("src/Controller/Uses.php");
    let source = fs::read(&path).unwrap();
    index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn certain_symfony_projects_flag_missing_names() {
    let (_dir, root) = broken_symfony(&[]);
    assert_eq!(
        broken_messages(&root),
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
    );
}

#[test]
fn unknown_route_imports_silence_route_diagnostics() {
    let (_dir, root) = broken_symfony(&[(
        "config/routes/acme.yaml",
        "acme:\n    resource: '@AcmeBundle/Resources/config/routes.yaml'\n",
    )]);
    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
}

#[test]
fn unreadable_route_names_silence_route_diagnostics() {
    let (_dir, root) = broken_symfony(&[
        (
            "lib/Route.php",
            "<?php\nnamespace Symfony\\Component\\Routing\\Attribute;\n#[\\Attribute]\nclass Route\n{\n    public function __construct(string $path = '', ?string $name = null) {}\n}\n",
        ),
        (
            "src/Controller/Named.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass Named\n{\n    const ROUTE = 'named';\n    #[Route('/named', name: self::ROUTE)]\n    public function show(): void {}\n}\n",
        ),
    ]);
    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
}

#[test]
fn php_route_files_silence_route_diagnostics() {
    let (_dir, root) = broken_symfony(&[("config/routes/extra.php", "<?php\n")]);
    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
}

#[test]
fn php_configuration_silences_parameter_diagnostics() {
    let (_dir, root) = broken_symfony(&[("config/services.php", "<?php\n")]);
    assert_eq!(broken_messages(&root), ["Undefined route 'missing_route'"]);
}

#[test]
fn computed_set_parameter_names_silence_parameter_diagnostics() {
    let (_dir, root) = broken_symfony(&[
        (
            "lib/ContainerBuilder.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection;\nclass ContainerBuilder\n{\n    public function setParameter(string $name, mixed $value): void {}\n}\n",
        ),
        (
            "src/Pass.php",
            "<?php\nnamespace App;\nuse Symfony\\Component\\DependencyInjection\\ContainerBuilder;\nclass Pass\n{\n    public function process(ContainerBuilder $container, string $name): void\n    {\n        $container->setParameter('app.literal', 1);\n        $container->setParameter($name, 1);\n    }\n}\n",
        ),
    ]);
    assert_eq!(broken_messages(&root), ["Undefined route 'missing_route'"]);
}

#[test]
fn diagnostics_stay_in_their_module() {
    let index = auto(&nested());
    // The Drupal route is not a Symfony route.
    assert_eq!(
        diagnostics_in(&index, "templates/demo.html.twig"),
        ["Undefined route 'demo.page'"]
    );
    assert!(
        diagnostics_in(
            &index,
            "codebase/drupal/web/modules/custom/demo/demo.services.yml"
        )
        .is_empty()
    );
    assert!(diagnostics_in(&index, "config/services.yaml").is_empty());
}

#[test]
fn the_compiled_container_is_not_configuration() {
    let (_dir, root) = broken_symfony(&[(
        "var/cache/dev/Container.php",
        "<?php\nnamespace ContainerX;\nuse Symfony\\Component\\Config\\Loader\\LoaderInterface;\nclass Container\n{\n    public function load(LoaderInterface $loader, $c, string $name): void\n    {\n        $c->setParameter($name, 1);\n    }\n}\n",
    )]);
    assert_eq!(
        broken_messages(&root),
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
    );
}

#[test]
fn annotation_route_imports_silence_route_diagnostics() {
    let (_dir, root) = broken_symfony(&[(
        "config/routes/annotations.yaml",
        "controllers:\n    resource: ../../src/Controller/\n    type: annotation\n",
    )]);
    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
}

#[test]
fn class_route_imports_count_only_for_project_classes() {
    let import = |class: &str| format!("acme:\n    resource: {class}\n    type: attribute\n");
    let vendor = import("Acme\\Auth\\LoginController");
    let (_dir, root) = broken_symfony(&[
        ("config/routes/acme.yaml", vendor.as_str()),
        (
            "vendor/acme/auth/LoginController.php",
            "<?php\nnamespace Acme\\Auth;\nclass LoginController {}\n",
        ),
    ]);
    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
    let project = import("App\\Controller\\Uses");
    let (_dir, root) = broken_symfony(&[("config/routes/uses.yaml", project.as_str())]);
    assert_eq!(broken_messages(&root).len(), 2);
}

#[test]
fn bundle_loaders_silence_parameter_diagnostics() {
    let (_dir, root) = broken_symfony(&[(
        "src/Billing/BillingExtension.php",
        "<?php\nnamespace App\\Billing;\nuse Symfony\\Component\\DependencyInjection\\Loader\\YamlFileLoader;\nclass BillingExtension\n{\n    public function load(YamlFileLoader $loader): void\n    {\n        $loader->load('services.yaml');\n    }\n}\n",
    )]);
    assert_eq!(broken_messages(&root), ["Undefined route 'missing_route'"]);
}

#[test]
fn environment_imports_silence_parameter_diagnostics() {
    let (_dir, root) = broken_symfony(&[(
        "config/packages/extra.yaml",
        "when@prod:\n    imports:\n        - { resource: extra_prod.yaml }\n",
    )]);
    assert_eq!(broken_messages(&root), ["Undefined route 'missing_route'"]);
}

#[test]
fn parameter_bag_sets_define_parameters() {
    let (_dir, root) = broken_symfony(&[
        (
            "lib/ContainerBuilder.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection;\nuse Symfony\\Component\\DependencyInjection\\ParameterBag\\ParameterBagInterface;\nclass ContainerBuilder\n{\n    public function getParameterBag(): ParameterBagInterface {}\n}\n",
        ),
        (
            "lib/ParameterBagInterface.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection\\ParameterBag;\ninterface ParameterBagInterface\n{\n    public function set(string $name, mixed $value): void;\n}\n",
        ),
        (
            "src/Pass.php",
            "<?php\nnamespace App;\nuse Symfony\\Component\\DependencyInjection\\ContainerBuilder;\nclass Pass\n{\n    public function process(ContainerBuilder $container): void\n    {\n        $container->getParameterBag()->set('app.nowhere', 1);\n    }\n}\n",
        ),
    ]);
    assert_eq!(broken_messages(&root), ["Undefined route 'missing_route'"]);
}

/// The diagnostics of `templates/page.html.twig` in a Symfony project with `files`,
/// where the page includes a template that exists nowhere.
fn template_messages(files: &[(&str, &str)]) -> Vec<String> {
    let mut all = vec![
        (
            "templates/page.html.twig",
            "{% include 'nowhere.html.twig' %}\n",
        ),
        (
            "vendor/symfony/twig-bridge/Resources/views/Form/form_div_layout.html.twig",
            "",
        ),
    ];
    all.extend_from_slice(files);
    let (_dir, root) = broken_symfony(&all);
    let index = auto(&root);
    let path = root.join("templates/page.html.twig");
    let source = fs::read(&path).unwrap();
    index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn unread_twig_configuration_silences_template_diagnostics() {
    assert_eq!(
        template_messages(&[]),
        ["Undefined template 'nowhere.html.twig'"]
    );
    let unread: [(&str, &str); 3] = [
        ("config/packages/twig.php", "<?php\n"),
        (
            "config/packages/views.yaml",
            "twig:\n    paths: ['extra']\n",
        ),
        (
            "config/packages/twig.yaml",
            "when@dev:\n    twig:\n        paths: ['%kernel.project_dir%/dev']\n",
        ),
    ];
    for file in unread {
        assert!(template_messages(&[file]).is_empty(), "{file:?}");
    }
}

const MODULE_HANDLER: &str = "<?php\nnamespace Drupal\\Core\\Extension;\ninterface ModuleHandlerInterface\n{\n    public function invokeAll($hook, array $args = []);\n}\n";

fn runner(namespace: &str) -> String {
    format!(
        "<?php\nnamespace {namespace};\nuse Drupal\\Core\\Extension\\ModuleHandlerInterface;\nclass Runner\n{{\n    public function go(ModuleHandlerInterface $handler): void\n    {{\n        $handler->invokeAll('cron');\n    }}\n}}\n"
    )
}

#[test]
fn hook_invocations_stay_in_the_drupal_project() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&nested(), &root);
    let drupal_dir = root.join("codebase/drupal/web");
    let files = [
        (
            drupal_dir.join("core/lib/Drupal/Core/Extension/ModuleHandlerInterface.php"),
            MODULE_HANDLER.to_string(),
        ),
        (
            drupal_dir.join("modules/custom/demo/demo.module"),
            "<?php\nfunction demo_cron() {}\n".to_string(),
        ),
        (
            drupal_dir.join("modules/custom/demo/src/Runner.php"),
            runner("Drupal\\demo"),
        ),
        (root.join("src/Runner.php"), runner("App")),
    ];
    for (path, content) in &files {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let mut index = auto(&root);
    let module = drupal_dir.join("modules/custom/demo/demo.module");
    let source = fs::read(&module).unwrap();
    let offset = String::from_utf8_lossy(&source).find("demo_cron").unwrap() as u32;

    let targets: Vec<PathBuf> = index
        .definition(&module, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect();

    assert_eq!(
        targets,
        vec![drupal_dir.join("modules/custom/demo/src/Runner.php")]
    );
}
