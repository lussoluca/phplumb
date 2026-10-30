use std::fs;
use std::path::{Path, PathBuf};

use phplumb_index::Index;
use phplumb_index::discover::DiscoverOptions;
use phplumb_index::frameworks::drupal::Drupal;
use phplumb_index::frameworks::symfony::{Symfony, TemplateStatus};
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
        linked: Vec::new(),
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
fn literal_constant_route_names_keep_routes_certain() {
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
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
    );
}

#[test]
fn constants_without_literal_values_keep_routes_uncertain() {
    let (_dir, root) = broken_symfony(&[
        (
            "lib/Route.php",
            "<?php\nnamespace Symfony\\Component\\Routing\\Attribute;\n#[\\Attribute]\nclass Route\n{\n    public function __construct(string $path = '', ?string $name = null) {}\n}\n",
        ),
        (
            "src/Controller/Named.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass Named\n{\n    const ROUTE = 'named';\n    const OTHER = self::ROUTE;\n    #[Route('/named', name: self::OTHER)]\n    public function show(): void {}\n}\n",
        ),
    ]);
    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
}

#[test]
fn empty_php_route_files_keep_routes_certain() {
    let (_dir, root) = broken_symfony(&[("config/routes/extra.php", "<?php\n")]);
    assert_eq!(
        broken_messages(&root),
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
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

#[cfg(unix)]
#[test]
fn symlinked_modules_belong_to_the_linking_project() {
    let (_dir, root) = project();
    lock(&root, ".", SYMFONY_LOCK);
    lock(&root, "drupal", DRUPAL_LOCK);
    let module = root.join("packages/foo");
    fs::create_dir_all(module.join("src")).unwrap();
    fs::write(module.join("foo.info.yml"), "name: Foo\ntype: module\n").unwrap();
    fs::write(
        module.join("foo.services.yml"),
        "services:\n  foo.svc:\n    class: Foo\\Svc\n",
    )
    .unwrap();
    fs::write(module.join("foo.module"), "<?php\nfunction foo_cron() {}\n").unwrap();
    fs::write(
        module.join("src/Svc.php"),
        "<?php\nnamespace Foo;\nclass Svc {}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("drupal/web/modules/custom")).unwrap();
    std::os::unix::fs::symlink(
        "../../../../packages/foo",
        root.join("drupal/web/modules/custom/foo"),
    )
    .unwrap();

    let index = auto(&root);

    assert_eq!(drupal(&index).services("foo.svc").len(), 1);
    let hooks: Vec<String> = drupal(&index)
        .hook_implementations("cron")
        .iter()
        .map(|i| i.name.clone())
        .collect();
    assert_eq!(hooks, vec!["foo_cron"]);
    let (_, excluded) = scope(&index, "symfony");
    assert!(excluded.contains(&module), "{excluded:?}");
}

#[cfg(unix)]
#[test]
fn links_outside_the_root_change_nothing() {
    let (_dir, root) = project();
    lock(&root, ".", SYMFONY_LOCK);
    lock(&root, "drupal", DRUPAL_LOCK);
    let outside = TempDir::new().unwrap();
    fs::create_dir_all(root.join("drupal/web/modules/custom")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("drupal/web/modules/custom/out")).unwrap();
    std::os::unix::fs::symlink("..", root.join("drupal/web/loop")).unwrap();

    let index = auto(&root);

    assert_eq!(scope(&index, "symfony").1, vec![root.join("drupal")]);
    assert!(index.framework_project("drupal").unwrap().linked.is_empty());
}

/// Go to definition at the first `needle` in `file`, below `root`.
fn definitions_at(index: &mut Index, root: &Path, file: &str, needle: &str) -> Vec<PathBuf> {
    let path = root.join(file);
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find(needle).unwrap() as u32;
    index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect()
}

/// A Symfony project at the root holding the full Drupal fixture in `drupal/`, with
/// Symfony files naming what the Drupal module would answer for.
fn symfony_with_drupal() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drupal"),
        &root.join("drupal"),
    );
    lock(&root, ".", SYMFONY_LOCK);
    for (file, content) in [
        (
            "src/app.services.yml",
            "services:\n  app.x:\n    arguments: ['@entity_type.manager']\n",
        ),
        ("src/hooks.php", "<?php\nfunction mymodule_example() {}\n"),
        (
            "src/Form.php",
            "<?php\nnamespace App;\nfunction form(): array\n{\n    return ['#type' => 'textfield'];\n}\n",
        ),
    ] {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    (dir, root)
}

#[test]
fn drupal_answers_nothing_in_symfony_yaml() {
    let (_dir, root) = symfony_with_drupal();
    let mut index = auto(&root);
    assert!(
        definitions_at(
            &mut index,
            &root,
            "src/app.services.yml",
            "entity_type.manager"
        )
        .is_empty()
    );
}

#[test]
fn drupal_declarations_stay_in_drupal() {
    let (_dir, root) = symfony_with_drupal();
    let mut index = auto(&root);

    assert_eq!(
        definitions_at(&mut index, &root, "src/hooks.php", "mymodule_example"),
        vec![root.join("src/hooks.php")]
    );
}

#[test]
fn drupal_keyed_strings_stay_in_drupal() {
    let (_dir, root) = symfony_with_drupal();
    let mut index = auto(&root);

    assert!(definitions_at(&mut index, &root, "src/Form.php", "textfield").is_empty());
}

#[test]
fn drupal_updates_ignore_yaml_outside_its_project() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_dir(&nested(), &root);
    let mut index = auto(&root);
    let outside = root.join("src/outside.services.yml");
    fs::write(
        &outside,
        "services:\n  outside.svc:\n    class: App\\Uses\n",
    )
    .unwrap();

    index.update(&[outside], &[]);

    assert!(drupal(&index).services("outside.svc").is_empty());
    assert!(!drupal(&index).defines_service(&index, "outside.svc"));
}

const LIB_CONTROLLER: &str = "<?php\nnamespace Lib;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass LibController\n{\n    #[Route('/x', name: 'lib_x')]\n    public function x(): void {}\n}\n";

#[cfg(unix)]
#[test]
fn packages_both_projects_link_stay_with_both() {
    let (_dir, root) = project();
    lock(&root, ".", SYMFONY_LOCK);
    lock(&root, "drupal", DRUPAL_LOCK);
    for (file, content) in [
        ("lib/AbstractController.php", ABSTRACT_CONTROLLER),
        (
            "src/Controller/Uses.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController;\nclass Uses extends AbstractController\n{\n    public function run(): void\n    {\n        $this->redirectToRoute('lib_x');\n    }\n}\n",
        ),
        ("config/routes.yaml", "home:\n    path: /\n"),
        ("packages/lib/src/LibController.php", LIB_CONTROLLER),
        (
            "packages/lib/config/services.yml",
            "services:\n  lib.svc:\n    class: Lib\\LibController\n",
        ),
    ] {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    fs::create_dir_all(root.join("vendor/acme")).unwrap();
    std::os::unix::fs::symlink("../../packages/lib", root.join("vendor/acme/lib")).unwrap();
    fs::create_dir_all(root.join("drupal/vendor/acme")).unwrap();
    std::os::unix::fs::symlink("../../../packages/lib", root.join("drupal/vendor/acme/lib"))
        .unwrap();
    let index = auto(&root);
    let path = root.join("src/Controller/Uses.php");
    let source = fs::read(&path).unwrap();

    let messages: Vec<String> = index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(messages.is_empty(), "{messages:?}");
    assert!(
        !index
            .framework::<Symfony>()
            .unwrap()
            .route_targets("lib_x")
            .is_empty()
    );
    assert!(drupal(&index).services("lib.svc").is_empty());
}

#[cfg(unix)]
#[test]
fn a_root_drupal_project_gets_modules_linked_from_a_nested_project() {
    let (_dir, root) = project();
    lock(&root, ".", DRUPAL_LOCK);
    lock(&root, "site", SYMFONY_LOCK);
    let module = root.join("site/bridge/foo");
    fs::create_dir_all(module.join("src")).unwrap();
    fs::write(module.join("foo.info.yml"), "name: Foo\ntype: module\n").unwrap();
    fs::write(
        module.join("foo.services.yml"),
        "services:\n  foo.svc:\n    class: Foo\\Svc\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("web/modules/custom")).unwrap();
    std::os::unix::fs::symlink(
        "../../../site/bridge/foo",
        root.join("web/modules/custom/foo"),
    )
    .unwrap();

    let index = auto(&root);

    assert_eq!(drupal(&index).services("foo.svc").len(), 1);
    assert!(
        !index
            .framework_project("symfony")
            .unwrap()
            .contains(&module.join("foo.services.yml"))
    );
}

#[cfg(unix)]
#[test]
fn links_to_another_project_leave_it_alone() {
    let (_dir, root) = project();
    lock(&root, "drupal", DRUPAL_LOCK);
    lock(&root, "site", SYMFONY_LOCK);
    fs::create_dir_all(root.join("site/src")).unwrap();
    fs::write(root.join("site/src/A.php"), "<?php\nclass A {}\n").unwrap();
    fs::create_dir_all(root.join("drupal/web/modules/custom")).unwrap();
    std::os::unix::fs::symlink(
        "../../../../site",
        root.join("drupal/web/modules/custom/site"),
    )
    .unwrap();

    let index = auto(&root);

    let file = root.join("site/src/A.php");
    assert!(index.framework_project("symfony").unwrap().contains(&file));
    assert!(!index.framework_project("drupal").unwrap().contains(&file));
}

fn route_files_of(index: &Index, name: &str) -> Vec<String> {
    index
        .framework::<Symfony>()
        .unwrap()
        .route_targets(name)
        .iter()
        .map(|target| target.text.clone())
        .collect()
}

const ROUTE_ATTRIBUTE: &str = "<?php\nnamespace Symfony\\Component\\Routing\\Attribute;\n#[\\Attribute]\nclass Route\n{\n    public function __construct(string $path = '', ?string $name = null) {}\n}\n";

#[test]
fn name_prefix_imports_prefix_their_attribute_routes() {
    let (_dir, root) = broken_symfony(&[
        ("lib/Route.php", ROUTE_ATTRIBUTE),
        (
            "config/routes.yaml",
            "home:\n    path: /\ncontrollers:\n    resource: ../src/Controller/\n    type: attribute\nadmin:\n    resource: ../src/Admin/\n    type: attribute\n    name_prefix: 'admin_'\n",
        ),
        (
            "src/Admin/PanelController.php",
            "<?php\nnamespace App\\Admin;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass PanelController\n{\n    #[Route('/panel', name: 'panel')]\n    public function panel(): void {}\n}\n",
        ),
        (
            "src/Controller/HomeController.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass HomeController\n{\n    #[Route('/start', name: 'start')]\n    public function start(): void {}\n}\n",
        ),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "admin_panel"), ["panel"]);
    assert!(route_files_of(&index, "admin_start").is_empty());
    assert_eq!(
        broken_messages(&root),
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
    );
}

#[test]
fn route_files_imported_from_outside_config_routes_are_read() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/routes.yaml",
            "home:\n    path: /\nblog:\n    resource: ../modules/blog/routes.yaml\n",
        ),
        ("modules/blog/routes.yaml", "blog_index:\n    path: /blog\n"),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "blog_index"), ["blog_index"]);
    assert_eq!(
        broken_messages(&root),
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
    );
}

const PHP_ROUTES: &str = "<?php\nuse App\\Controller\\Uses;\nuse Symfony\\Component\\Routing\\Loader\\Configurator\\RoutingConfigurator;\nreturn static function (RoutingConfigurator $routes): void {\n    $routes->add('php_home', '/')->methods(['GET'])->controller([Uses::class, 'run']);\n};\n";

#[test]
fn php_route_files_define_their_routes() {
    let (_dir, root) = broken_symfony(&[("config/routes/app.php", PHP_ROUTES)]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "php_home"), ["php_home", "run"]);
    assert_eq!(
        broken_messages(&root),
        [
            "Undefined route 'missing_route'",
            "Undefined parameter 'app.nowhere'"
        ]
    );
}

#[test]
fn php_route_imports_and_computed_names_silence_route_diagnostics() {
    for content in [
        "<?php\nreturn static function ($routes): void {\n    $routes->import('../src/Other/', 'attribute');\n};\n",
        "<?php\nreturn static function ($routes): void {\n    $name = 'x';\n    $routes->add($name, '/x');\n};\n",
    ] {
        let (_dir, root) = broken_symfony(&[("config/routes/extra.php", content)]);
        assert_eq!(
            broken_messages(&root),
            ["Undefined parameter 'app.nowhere'"],
            "{content}"
        );
    }
}

#[test]
fn route_files_follow_the_kernel_patterns() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/routes/deep/nested/x.yaml",
            "deep_x:\n    path: /x\n",
        ),
        ("config/routes/y.yml", "yml_y:\n    path: /y\n"),
        ("config/routes/dev/z.yaml", "dev_z:\n    path: /z\n"),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "dev_z"), ["dev_z"]);
    assert!(route_files_of(&index, "deep_x").is_empty());
    assert!(route_files_of(&index, "yml_y").is_empty());
}

#[cfg(unix)]
#[test]
fn route_file_symlink_loops_read_each_file_once() {
    let (_dir, root) = broken_symfony(&[("config/routes/a.yaml", "a_route:\n    path: /a\n")]);
    std::os::unix::fs::symlink("..", root.join("config/routes/loop")).unwrap();
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "home"), ["home"]);
    assert_eq!(route_files_of(&index, "a_route"), ["a_route"]);
}

#[test]
fn route_file_updates_read_extensions_in_any_case() {
    let (_dir, root) = broken_symfony(&[]);
    let mut index = auto(&root);
    let upper = root.join("config/routes/A.YAML");
    fs::create_dir_all(upper.parent().unwrap()).unwrap();
    fs::write(&upper, "upper_a:\n    path: /a\n").unwrap();

    index.update(&[upper], &[]);

    assert_eq!(route_files_of(&index, "upper_a"), ["upper_a"]);
}

#[test]
fn routes_php_is_read_only_without_routes_yaml() {
    let (_dir, root) = broken_symfony(&[("config/routes.php", PHP_ROUTES)]);
    assert!(route_files_of(&auto(&root), "php_home").is_empty());
    fs::remove_file(root.join("config/routes.yaml")).unwrap();
    assert_eq!(
        route_files_of(&auto(&root), "php_home"),
        ["php_home", "run"]
    );
}

/// The diagnostics of `broken_symfony` with `files` and the route attribute class.
fn route_messages(files: &[(&str, &str)]) -> Vec<String> {
    let mut all = vec![("lib/Route.php", ROUTE_ATTRIBUTE)];
    all.extend_from_slice(files);
    let (_dir, root) = broken_symfony(&all);
    broken_messages(&root)
}

const CERTAIN: [&str; 2] = [
    "Undefined route 'missing_route'",
    "Undefined parameter 'app.nowhere'",
];

#[test]
fn prefixed_import_cycles_read_each_file_once() {
    let (_dir, root) = broken_symfony(&[(
        "config/routes/a.yaml",
        "a_route:\n    path: /a\nagain:\n    resource: a.yaml\n    name_prefix: 'x_'\n",
    )]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "a_route"), ["a_route"]);
}

#[test]
fn php_route_files_in_other_forms_silence_route_diagnostics() {
    for content in [
        "<?php\nuse Symfony\\Component\\Routing\\Loader\\Configurator\\RoutingConfigurator;\nreturn static function (RoutingConfigurator $routes): void {\n    $routes('inv_home', '/inv');\n};\n",
        "<?php\nnamespace Symfony\\Component\\Routing\\Loader\\Configurator;\nreturn Routes::config([\n    'arr_home' => ['path' => '/arr'],\n]);\n",
        "<?php\nuse Symfony\\Component\\Routing\\Route;\nuse Symfony\\Component\\Routing\\RouteCollection;\n$c = new RouteCollection();\n$c->add('n', new Route('/n'));\n$c->addNamePrefix('p_');\nreturn $c;\n",
        "<?php\nreturn static function ($routes): void {\n    if (true) {\n        $routes->add('cond', '/cond');\n    }\n};\n",
    ] {
        let (_dir, root) = broken_symfony(&[("config/routes/app.php", content)]);
        assert_eq!(
            broken_messages(&root),
            ["Undefined parameter 'app.nowhere'"],
            "{content}"
        );
    }
}

#[test]
fn unreadable_aliases_silence_route_diagnostics() {
    let messages = route_messages(&[(
        "src/Controller/A.php",
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nuse Symfony\\Component\\Routing\\Attribute\\DeprecatedAlias;\nclass A\n{\n    #[Route('/a', name: 'a', alias: [new DeprecatedAlias('old_a', 'pkg', '1.0')])]\n    public function a(): void {}\n}\n",
    )]);

    assert_eq!(messages, ["Undefined parameter 'app.nowhere'"]);
}

#[test]
fn constant_aliases_lead_to_their_route() {
    let files = [(
        "src/Controller/B.php",
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass B\n{\n    const OLD = 'old_b';\n    #[Route('/b', name: 'b', alias: self::OLD)]\n    public function b(): void {}\n}\n",
    )];
    let mut all = vec![("lib/Route.php", ROUTE_ATTRIBUTE)];
    all.extend_from_slice(&files);
    let (_dir, root) = broken_symfony(&all);

    assert_eq!(route_files_of(&auto(&root), "old_b"), ["b"]);
    assert_eq!(broken_messages(&root), CERTAIN);
}

#[test]
fn class_aliases_of_invokable_classes_lead_to_their_route() {
    let (_dir, root) = broken_symfony(&[
        ("lib/Route.php", ROUTE_ATTRIBUTE),
        (
            "src/Controller/Inv.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\n#[Route('/inv', name: 'inv', alias: 'inv_old')]\nclass Inv\n{\n    public function __invoke(): void {}\n}\n",
        ),
    ]);

    assert_eq!(route_files_of(&auto(&root), "inv_old"), ["__invoke"]);
    assert_eq!(broken_messages(&root), CERTAIN);
}

#[test]
fn outer_name_prefixes_reach_nested_imports() {
    let (_dir, root) = broken_symfony(&[
        ("lib/Route.php", ROUTE_ATTRIBUTE),
        (
            "config/routes.yaml",
            "home:\n    path: /\nblog:\n    resource: ../modules/blog/routes.yaml\n    name_prefix: 'blog_'\n",
        ),
        (
            "modules/blog/routes.yaml",
            "ctrl:\n    resource: ../../src/Blog/\n    type: attribute\ninner:\n    resource: ../../config/routes/k.yaml\n",
        ),
        ("config/routes/k.yaml", "k:\n    path: /k\n"),
        (
            "src/Blog/PostController.php",
            "<?php\nnamespace App\\Blog;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass PostController\n{\n    #[Route('/post', name: 'post')]\n    public function post(): void {}\n}\n",
        ),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "blog_post"), ["post"]);
    assert_eq!(route_files_of(&index, "blog_k"), ["k"]);
    assert_eq!(broken_messages(&root), CERTAIN);
}

#[test]
fn directory_imports_read_every_route_file_below() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/routes.yaml",
            "home:\n    path: /\nlegacy:\n    resource: 'routes/legacy/'\n    type: directory\n",
        ),
        ("config/routes/legacy/old/x.yaml", "deep_x:\n    path: /x\n"),
        ("config/routes/legacy/y.yml", "yml_y:\n    path: /y\n"),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "deep_x"), ["deep_x"]);
    assert_eq!(route_files_of(&index, "yml_y"), ["yml_y"]);
    assert_eq!(broken_messages(&root), CERTAIN);
}

#[test]
fn directory_imports_with_unread_files_silence_route_diagnostics() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/routes.yaml",
            "home:\n    path: /\nlegacy:\n    resource: '../modules/legacy/'\n    type: directory\n",
        ),
        ("modules/legacy/x.xml", "<routes/>\n"),
    ]);

    assert_eq!(
        broken_messages(&root),
        ["Undefined parameter 'app.nowhere'"]
    );
}

#[test]
fn inherited_routes_read_constants_of_their_declaring_class() {
    let (_dir, root) = broken_symfony(&[
        ("lib/Route.php", ROUTE_ATTRIBUTE),
        (
            "src/Controller/Base.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nabstract class Base\n{\n    const ROUTE = 'base_r';\n    #[Route('/b', name: self::ROUTE)]\n    public function show(): void {}\n}\n",
        ),
        (
            "src/Controller/Child.php",
            "<?php\nnamespace App\\Controller;\nclass Child extends Base\n{\n    const ROUTE = 'child_r';\n}\n",
        ),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "base_r"), ["show"]);
    assert!(route_files_of(&index, "child_r").is_empty());
}

#[test]
fn single_locale_routes_keep_their_class_alias() {
    let (_dir, root) = broken_symfony(&[
        ("lib/Route.php", ROUTE_ATTRIBUTE),
        (
            "src/Controller/Loc.php",
            "<?php\nnamespace App\\Controller;\nuse Symfony\\Component\\Routing\\Attribute\\Route;\nclass Loc\n{\n    #[Route(path: ['en' => '/about'], name: 'about')]\n    public function about(): void {}\n}\n",
        ),
    ]);

    assert_eq!(
        route_files_of(&auto(&root), "App\\Controller\\Loc::about"),
        ["about"]
    );
}

#[test]
fn prefixed_aliases_lead_to_their_prefixed_route() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/routes.yaml",
            "home:\n    path: /\nindex:\n    path: /other\nblog:\n    resource: ../modules/blog/routes.yaml\n    name_prefix: 'blog_'\n",
        ),
        (
            "modules/blog/routes.yaml",
            "index:\n    path: /blog\nold_index:\n    alias: index\n",
        ),
    ]);
    let index = auto(&root);
    let symfony = index.framework::<Symfony>().unwrap();

    assert_eq!(
        symfony.route_targets("blog_old_index"),
        symfony.route_targets("blog_index")
    );
}

#[test]
fn alias_chains_lead_to_their_route() {
    let (_dir, root) = broken_symfony(&[(
        "config/routes/aliases.yaml",
        "second:\n    alias: first\nfirst:\n    alias: home\n",
    )]);

    assert_eq!(route_files_of(&auto(&root), "second"), ["home"]);
    assert_eq!(broken_messages(&root), CERTAIN);
}

const BUNDLES: &str = "<?php\nreturn [\n    Symfony\\Bundle\\FrameworkBundle\\FrameworkBundle::class => ['all' => true],\n    Acme\\AcmeBundle::class => ['all' => true],\n];\n";
const BUNDLE: &str = "<?php\nnamespace Symfony\\Component\\HttpKernel\\Bundle;\nabstract class Bundle\n{\n    public function getPath(): string { return ''; }\n}\n";
const ABSTRACT_BUNDLE: &str = "<?php\nnamespace Symfony\\Component\\HttpKernel\\Bundle;\nabstract class AbstractBundle extends Bundle\n{\n    public function getPath(): string { return ''; }\n}\n";
const FRAMEWORK_BUNDLE: &str = "<?php\nnamespace Symfony\\Bundle\\FrameworkBundle;\nuse Symfony\\Component\\HttpKernel\\Bundle\\Bundle;\nclass FrameworkBundle extends Bundle {}\n";
const ERRORS_ROUTES: &str = "<?php\nuse Symfony\\Component\\Routing\\Loader\\Configurator\\RoutingConfigurator;\nreturn static function (RoutingConfigurator $routes): void {\n    $routes->add('_preview_error', '/{code}.{_format}')\n        ->controller('error_controller::preview');\n};\n";
const ACME_BUNDLE: &str = "<?php\nnamespace Acme;\nuse Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle;\nclass AcmeBundle extends AbstractBundle {}\n";

/// A Symfony project with `config/bundles.php`, Symfony's FrameworkBundle and the
/// `AcmeBundle`, an `AbstractBundle` defining `acme_home`, and `files`.
fn bundled_symfony(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let mut all = vec![
        ("config/bundles.php", BUNDLES),
        ("lib/Bundle.php", BUNDLE),
        ("lib/AbstractBundle.php", ABSTRACT_BUNDLE),
        ("lib/framework-bundle/FrameworkBundle.php", FRAMEWORK_BUNDLE),
        (
            "lib/framework-bundle/Resources/config/routing/errors.php",
            ERRORS_ROUTES,
        ),
        ("lib/acme/src/AcmeBundle.php", ACME_BUNDLE),
        (
            "lib/acme/config/routes.yaml",
            "acme_home:\n    path: /acme\n",
        ),
    ];
    all.extend_from_slice(files);
    broken_symfony(&all)
}

fn route_missing(index: &Index, name: &str) -> bool {
    index.framework::<Symfony>().unwrap().route_missing(name)
}

#[test]
fn bundle_routes_lead_to_their_definitions() {
    let (_dir, root) = bundled_symfony(&[(
        "config/routes/framework.yaml",
        "when@dev:\n    _errors:\n        resource: '@FrameworkBundle/Resources/config/routing/errors.php'\n        prefix: /_error\n",
    )]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "_preview_error"), ["_preview_error"]);
    assert!(route_missing(&index, "_missing"));
}

#[test]
fn abstract_bundles_read_routes_from_their_parent_directory() {
    let (_dir, root) = bundled_symfony(&[(
        "config/routes/acme.yaml",
        "acme:\n    resource: '@AcmeBundle/config/routes.yaml'\n",
    )]);

    assert_eq!(route_files_of(&auto(&root), "acme_home"), ["acme_home"]);
    assert_eq!(broken_messages(&root), CERTAIN);
}

#[test]
fn prefixed_bundle_imports_prefix_bundle_routes() {
    let (_dir, root) = bundled_symfony(&[(
        "config/routes/acme.yaml",
        "acme:\n    resource: '@AcmeBundle/config/routes.yaml'\n    name_prefix: 'x_'\n",
    )]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "x_acme_home"), ["acme_home"]);
    assert!(route_files_of(&index, "acme_home").is_empty());
}

#[test]
fn unresolved_bundles_keep_routes_uncertain() {
    let unregistered = [(
        "config/routes/other.yaml",
        "other:\n    resource: '@OtherBundle/config/routes.yaml'\n",
    )];
    let own_path = [
        (
            "lib/acme/src/AcmeBundle.php",
            "<?php\nnamespace Acme;\nuse Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle;\nclass AcmeBundle extends AbstractBundle\n{\n    public function getPath(): string { return __DIR__; }\n}\n",
        ),
        (
            "config/routes/acme.yaml",
            "acme:\n    resource: '@AcmeBundle/config/routes.yaml'\n",
        ),
    ];
    for files in [&unregistered[..], &own_path[..]] {
        let (_dir, root) = bundled_symfony(files);
        assert_eq!(
            broken_messages(&root),
            ["Undefined parameter 'app.nowhere'"],
            "{files:?}"
        );
    }
}

#[test]
fn logout_routes_lead_to_the_firewall() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/packages/security.yaml",
            "security:\n    firewalls:\n        dev:\n            security: false\n        main:\n            logout:\n                path: /logout\n",
        ),
        (
            "config/routes/security.yaml",
            "_security_logout:\n    resource: security.route_loader.logout\n    type: service\n",
        ),
    ]);
    let index = auto(&root);

    assert_eq!(route_files_of(&index, "_logout_main"), ["logout"]);
    assert!(route_files_of(&index, "_logout_dev").is_empty());
    assert!(!route_missing(&index, "_missing"));
}

#[test]
fn named_route_arguments_are_flagged() {
    let (_dir, root) = broken_symfony(&[(
        "src/Controller/Uses.php",
        "<?php\nnamespace App\\Controller;\nuse Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController;\nclass Uses extends AbstractController\n{\n    public function run(): void\n    {\n        $this->redirectToRoute(route: 'missing_named');\n        $this->redirectToRoute(route: 'home');\n    }\n}\n",
    )]);

    assert_eq!(broken_messages(&root), ["Undefined route 'missing_named'"]);
}

#[test]
fn logout_routes_need_a_logout_path() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/packages/security.yaml",
            "security:\n    firewalls:\n        main:\n            logout:\n                path: app_logout\n        admin:\n            logout: ~\n",
        ),
        (
            "config/routes/security.yaml",
            "_security_logout:\n    resource: security.route_loader.logout\n    type: service\n",
        ),
    ]);
    let index = auto(&root);

    assert!(route_files_of(&index, "_logout_main").is_empty());
    assert_eq!(route_files_of(&index, "_logout_admin"), ["logout"]);
}

#[test]
fn kernel_abstract_bundles_read_routes_from_their_parent_directory() {
    let (_dir, root) = bundled_symfony(&[
        (
            "lib/KernelAbstractBundle.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection\\Kernel;\nabstract class AbstractBundle\n{\n    public function getPath(): string { return ''; }\n}\n",
        ),
        (
            "lib/acme/src/AcmeBundle.php",
            "<?php\nnamespace Acme;\nuse Symfony\\Component\\DependencyInjection\\Kernel\\AbstractBundle;\nclass AcmeBundle extends AbstractBundle {}\n",
        ),
        (
            "config/routes/acme.yaml",
            "acme:\n    resource: '@AcmeBundle/config/routes.yaml'\n",
        ),
    ]);

    assert_eq!(route_files_of(&auto(&root), "acme_home"), ["acme_home"]);
}

#[test]
fn bundles_built_on_kernel_abstract_bundles_keep_their_own_path() {
    let (_dir, root) = bundled_symfony(&[
        (
            "lib/KernelAbstractBundle.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection\\Kernel;\nabstract class AbstractBundle\n{\n    public function getPath(): string { return ''; }\n}\n",
        ),
        (
            "lib/Bundle.php",
            "<?php\nnamespace Symfony\\Component\\HttpKernel\\Bundle;\nuse Symfony\\Component\\DependencyInjection\\Kernel\\AbstractBundle as BaseAbstractBundle;\nabstract class Bundle extends BaseAbstractBundle\n{\n    public function getPath(): string { return ''; }\n}\n",
        ),
        (
            "config/routes/framework.yaml",
            "_errors:\n    resource: '@FrameworkBundle/Resources/config/routing/errors.php'\n",
        ),
    ]);

    assert_eq!(
        route_files_of(&auto(&root), "_preview_error"),
        ["_preview_error"]
    );
}

/// TwigBridge's form themes, so the main namespace can be known in full.
const TWIG_BRIDGE: (&str, &str) = (
    "vendor/symfony/twig-bridge/Resources/views/Form/form_div_layout.html.twig",
    "form\n",
);

fn template_status(root: &Path, name: &str) -> TemplateStatus {
    auto(root)
        .framework::<Symfony>()
        .unwrap()
        .template_status(name)
}

#[test]
fn environment_twig_files_add_namespaced_paths() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/packages/dev/twig.yaml",
            "twig:\n    paths:\n        '%kernel.project_dir%/dev/templates': Dev\n",
        ),
        ("dev/templates/tools.html.twig", "tools\n"),
        ("templates/base.html.twig", "base\n"),
        TWIG_BRIDGE,
    ]);

    assert!(matches!(
        template_status(&root, "@Dev/tools.html.twig"),
        TemplateStatus::Found(_)
    ));
    assert_eq!(
        template_status(&root, "gone.html.twig"),
        TemplateStatus::Missing
    );
}

#[test]
fn environment_twig_files_with_main_paths_keep_main_names_unknown() {
    for twig in [
        "twig:\n    paths:\n        '%kernel.project_dir%/dev/main': ~\n",
        "twig:\n    default_path: '%kernel.project_dir%/dev/main'\n",
    ] {
        let (_dir, root) = broken_symfony(&[
            ("config/packages/dev/twig.yaml", twig),
            ("templates/base.html.twig", "base\n"),
            TWIG_BRIDGE,
        ]);
        assert_eq!(
            template_status(&root, "gone.html.twig"),
            TemplateStatus::Unknown,
            "{twig}"
        );
    }
}

#[test]
fn environment_twig_php_keeps_names_unknown() {
    let (_dir, root) = broken_symfony(&[
        ("config/packages/dev/twig.php", "<?php\nreturn [];\n"),
        ("templates/base.html.twig", "base\n"),
        TWIG_BRIDGE,
    ]);

    assert_eq!(
        template_status(&root, "gone.html.twig"),
        TemplateStatus::Unknown
    );
}

fn template_file(root: &Path, name: &str) -> Option<String> {
    let found = auto(root)
        .framework::<Symfony>()
        .unwrap()
        .template_file(name)?;
    Some(found.strip_prefix(root).unwrap().display().to_string())
}

#[test]
fn list_and_relative_twig_paths_are_read() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/packages/twig.yaml",
            "twig:\n    paths: ['%kernel.project_dir%/listed', 'relative']\n",
        ),
        ("listed/a.html.twig", "a\n"),
        ("relative/b.html.twig", "b\n"),
    ]);

    assert_eq!(
        template_file(&root, "a.html.twig").as_deref(),
        Some("listed/a.html.twig")
    );
    assert_eq!(
        template_file(&root, "b.html.twig").as_deref(),
        Some("relative/b.html.twig")
    );
}

#[test]
fn default_path_overrides_templates() {
    let (_dir, root) = broken_symfony(&[
        (
            "config/packages/twig.yaml",
            "twig:\n    default_path: '%kernel.project_dir%/views'\n",
        ),
        ("views/v.html.twig", "v\n"),
        ("templates/t.html.twig", "t\n"),
        TWIG_BRIDGE,
    ]);

    assert_eq!(
        template_file(&root, "v.html.twig").as_deref(),
        Some("views/v.html.twig")
    );
    assert_eq!(
        template_status(&root, "t.html.twig"),
        TemplateStatus::Missing
    );
}

#[test]
fn twig_files_of_a_nested_drupal_project_get_no_symfony_answers() {
    let (_dir, root) = symfony_with_drupal();
    for (file, content) in [
        ("templates/base.html.twig", "base\n"),
        (
            "drupal/templates/page.html.twig",
            "{% include 'base.html.twig' %}\n{% include 'gone.html.twig' %}\n",
        ),
    ] {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let mut index = auto(&root);
    let path = root.join("drupal/templates/page.html.twig");
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find("'base").unwrap() as u32 + 1;

    assert!(index.definition(&path, &source, offset).is_empty());
    assert!(index.diagnostics(&path, &source).is_empty());
    let own = root.join("templates/base.html.twig");
    assert!(
        index
            .framework::<Symfony>()
            .unwrap()
            .template_file("base.html.twig")
            == Some(own)
    );
}

/// `bundled_symfony` with templates in both bundles, the project's templates and
/// TwigBridge, and `files`.
fn templated_bundles(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let mut all = vec![
        ("lib/framework-bundle/Resources/views/x.html.twig", "x\n"),
        (
            "lib/acme/templates/card.html.twig",
            "{% include 'nowhere.html.twig' %}\n",
        ),
        (
            "templates/page.html.twig",
            "{% include 'nowhere.html.twig' %}\n",
        ),
        TWIG_BRIDGE,
    ];
    all.extend_from_slice(files);
    bundled_symfony(&all)
}

#[test]
fn bundle_templates_open_from_their_views() {
    let (_dir, root) = templated_bundles(&[]);

    assert_eq!(
        template_file(&root, "@Framework/x.html.twig").as_deref(),
        Some("lib/framework-bundle/Resources/views/x.html.twig")
    );
    assert_eq!(
        template_file(&root, "@Acme/card.html.twig").as_deref(),
        Some("lib/acme/templates/card.html.twig")
    );
    assert_eq!(
        template_status(&root, "@Acme/gone.html.twig"),
        TemplateStatus::Missing
    );
}

#[test]
fn bundle_overrides_come_first() {
    let (_dir, root) =
        templated_bundles(&[("templates/bundles/AcmeBundle/card.html.twig", "own\n")]);

    assert_eq!(
        template_file(&root, "@Acme/card.html.twig").as_deref(),
        Some("templates/bundles/AcmeBundle/card.html.twig")
    );
    assert_eq!(
        template_file(&root, "@!Acme/card.html.twig").as_deref(),
        Some("lib/acme/templates/card.html.twig")
    );
}

#[test]
fn unresolved_bundle_namespaces_stay_unknown() {
    let own_path = [(
        "lib/acme/src/AcmeBundle.php",
        "<?php\nnamespace Acme;\nuse Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle;\nclass AcmeBundle extends AbstractBundle\n{\n    public function getPath(): string { return __DIR__; }\n}\n",
    )];
    let environment_default = [(
        "config/packages/twig.yaml",
        "when@dev:\n    twig:\n        default_path: '%kernel.project_dir%/dev_templates'\n",
    )];
    for files in [&own_path[..], &environment_default[..]] {
        let (_dir, root) = templated_bundles(files);
        for name in ["@Acme/card.html.twig", "@Acme/gone.html.twig"] {
            assert_eq!(
                template_status(&root, name),
                TemplateStatus::Unknown,
                "{files:?} {name}"
            );
        }
    }
}

#[test]
fn bundle_templates_get_no_diagnostics() {
    let (_dir, root) = templated_bundles(&[]);
    let index = auto(&root);
    let messages = |file: &str| -> Vec<String> {
        let path = root.join(file);
        let source = fs::read(&path).unwrap();
        index
            .diagnostics(&path, &source)
            .into_iter()
            .map(|d| d.message)
            .collect()
    };

    assert!(messages("lib/acme/templates/card.html.twig").is_empty());
    assert_eq!(
        messages("templates/page.html.twig"),
        ["Undefined template 'nowhere.html.twig'"]
    );
}

#[test]
fn project_bundles_keep_project_template_diagnostics() {
    let (_dir, root) = templated_bundles(&[
        (
            "config/bundles.php",
            "<?php\nreturn [\n    App\\AppBundle::class => ['all' => true],\n];\n",
        ),
        (
            "src/AppBundle.php",
            "<?php\nnamespace App;\nuse Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle;\nclass AppBundle extends AbstractBundle {}\n",
        ),
    ]);
    let index = auto(&root);
    let path = root.join("templates/page.html.twig");
    let source = fs::read(&path).unwrap();
    let messages: Vec<String> = index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|d| d.message)
        .collect();

    assert_eq!(messages, ["Undefined template 'nowhere.html.twig'"]);
}

#[test]
fn bundles_setting_their_path_stay_unknown() {
    let (_dir, root) = templated_bundles(&[
        (
            "lib/acme/src/AcmeBundle.php",
            "<?php\nnamespace Acme;\nuse Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle;\nclass AcmeBundle extends AbstractBundle\n{\n    public function __construct()\n    {\n        $this->path = \\dirname(__DIR__).'/res';\n    }\n}\n",
        ),
        ("lib/acme/res/templates/real.html.twig", "real\n"),
        (
            "config/routes/acme.yaml",
            "acme:\n    resource: '@AcmeBundle/config/routes.yaml'\n",
        ),
    ]);

    assert_eq!(
        template_status(&root, "@Acme/real.html.twig"),
        TemplateStatus::Unknown
    );
    assert_eq!(
        template_status(&root, "@Acme/card.html.twig"),
        TemplateStatus::Unknown
    );
    assert!(route_files_of(&auto(&root), "acme_home").is_empty());
    assert!(!route_missing(&auto(&root), "nope"));
}

#[test]
fn bare_attributes_of_other_classes_on_hooks_open_their_class() {
    let (_dir, root) = symfony_with_drupal();
    let module = root.join("drupal/modules/custom/mymodule/mymodule.module");
    let text = fs::read_to_string(&module).unwrap();
    fs::write(
        &module,
        text.replace(
            "function /*@cursor:ping_impl*/mymodule_ping(",
            "#[\\Drupal\\mymodule\\Traced]\nfunction /*@cursor:ping_impl*/mymodule_ping(",
        ),
    )
    .unwrap();
    fs::write(
        root.join("drupal/modules/custom/mymodule/src/Traced.php"),
        "<?php\nnamespace Drupal\\mymodule;\n#[\\Attribute]\nfinal class Traced\n{\n}\n",
    )
    .unwrap();
    let mut index = auto(&root);
    let source = fs::read(&module).unwrap();
    let offset = String::from_utf8_lossy(&source).find("Traced]").unwrap() as u32 + 1;

    let found: Vec<PathBuf> = index
        .definition(&module, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect();
    assert_eq!(
        found,
        [root.join("drupal/modules/custom/mymodule/src/Traced.php")]
    );
}

const AS_MESSAGE_HANDLER: &str = "<?php\nnamespace Symfony\\Component\\Messenger\\Attribute;\n#[\\Attribute]\nclass AsMessageHandler\n{\n    public function __construct(?string $bus = null, ?string $fromTransport = null, ?string $handles = null, ?string $method = null) {}\n}\n";
const MESSAGE_BUS: &str = "<?php\nnamespace Symfony\\Component\\Messenger;\ninterface MessageBusInterface\n{\n    public function dispatch(object $message, array $stamps = []): object;\n}\n";
const PING: &str = "<?php\nnamespace App\\Message;\nfinal class Ping\n{\n}\n";
const PING_HANDLER: &str = "<?php\nnamespace App\\Handler;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Messenger\\Attribute\\AsMessageHandler;\n#[AsMessageHandler]\nfinal class PingHandler\n{\n    public function __invoke(Ping $message): void\n    {\n    }\n}\n";
const SENDER: &str = "<?php\nnamespace App\\Service;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Messenger\\MessageBusInterface;\nfinal class Sender\n{\n    public function __construct(private MessageBusInterface $bus)\n    {\n    }\n\n    public function send(): void\n    {\n        $this->bus->dispatch(new Ping());\n    }\n}\n";

/// `broken_symfony` with Messenger's classes, a `Ping` message, its handler, a
/// sender, and `files`.
fn messenger_symfony(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
    let mut all = vec![
        ("lib/AsMessageHandler.php", AS_MESSAGE_HANDLER),
        ("lib/MessageBusInterface.php", MESSAGE_BUS),
        ("src/Message/Ping.php", PING),
        ("src/Handler/PingHandler.php", PING_HANDLER),
        ("src/Service/Sender.php", SENDER),
    ];
    all.extend_from_slice(files);
    broken_symfony(&all)
}

/// The definitions at the first `needle` in `file` below `root`, as (file name,
/// text) pairs, sorted.
fn handler_definitions(
    index: &mut Index,
    root: &Path,
    file: &str,
    needle: &str,
) -> Vec<(String, String)> {
    let path = root.join(file);
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find(needle).unwrap() as u32 + 1;
    let mut found: Vec<(String, String)> = index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| {
            let text = fs::read(&location.path).unwrap();
            (
                location
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                String::from_utf8_lossy(&text[location.start as usize..location.end as usize])
                    .into_owned(),
            )
        })
        .collect();
    found.sort();
    found
}

fn named(file: &str, text: &str) -> (String, String) {
    (file.to_string(), text.to_string())
}

#[cfg(unix)]
#[test]
fn handlers_under_a_symlinked_root_list_their_dispatches() {
    let (_dir, root) = messenger_symfony(&[]);
    let links = TempDir::new().unwrap();
    let link = links.path().join("workspace");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    let mut index = auto(&link);

    assert_eq!(
        handler_definitions(&mut index, &link, "src/Handler/PingHandler.php", "__invoke"),
        [named("Sender.php", "dispatch")]
    );
    assert_eq!(
        handler_definitions(&mut index, &link, "src/Service/Sender.php", "dispatch("),
        [named("PingHandler.php", "__invoke")]
    );
}

const WIDE: &str = "<?php\nnamespace App\\Service;\nuse App\\Message\\Sub\\Deep;\nuse App\\MessageX;\nuse Symfony\\Component\\Messenger\\MessageBusInterface;\nfinal class Wide\n{\n    public function __construct(private MessageBusInterface $bus)\n    {\n    }\n\n    public function deep(): void\n    {\n        $this->bus->dispatch(new Deep());\n    }\n\n    public function other(): void\n    {\n        $this->bus->dispatch(new MessageX());\n    }\n}\n";

fn wildcard_symfony(handles: &str) -> (TempDir, PathBuf) {
    let handler = format!(
        "<?php\nnamespace App\\Handler;\nuse Symfony\\Component\\Messenger\\Attribute\\AsMessageHandler;\nfinal class WideHandler\n{{\n    #[AsMessageHandler(handles: '{handles}')]\n    public function onAny(object $message): void\n    {{\n    }}\n}}\n"
    );
    let handler: &'static str = Box::leak(handler.into_boxed_str());
    messenger_symfony(&[
        (
            "src/Message/Sub/Deep.php",
            "<?php\nnamespace App\\Message\\Sub;\nfinal class Deep\n{\n}\n",
        ),
        (
            "src/MessageX.php",
            "<?php\nnamespace App;\nfinal class MessageX\n{\n}\n",
        ),
        ("src/Service/Wide.php", WIDE),
        ("src/Handler/WideHandler.php", handler),
    ])
}

#[test]
fn wildcard_handlers_handle_every_message() {
    let (_dir, root) = wildcard_symfony("*");
    let mut index = auto(&root);

    for needle in ["dispatch(new Deep", "dispatch(new MessageX"] {
        assert!(
            handler_definitions(&mut index, &root, "src/Service/Wide.php", needle)
                .contains(&named("WideHandler.php", "onAny")),
            "{needle}"
        );
    }
    assert_eq!(
        handler_definitions(&mut index, &root, "src/Handler/WideHandler.php", "onAny"),
        [
            named("Sender.php", "dispatch"),
            named("Wide.php", "dispatch"),
            named("Wide.php", "dispatch")
        ]
    );
}

#[test]
fn namespace_wildcard_handlers_handle_their_namespace() {
    let (_dir, root) = wildcard_symfony("App\\\\Message\\\\*");
    let mut index = auto(&root);

    assert_eq!(
        handler_definitions(
            &mut index,
            &root,
            "src/Service/Wide.php",
            "dispatch(new Deep"
        ),
        [named("WideHandler.php", "onAny")]
    );
    assert!(
        !handler_definitions(
            &mut index,
            &root,
            "src/Service/Wide.php",
            "dispatch(new MessageX"
        )
        .contains(&named("WideHandler.php", "onAny"))
    );
    assert_eq!(
        handler_definitions(&mut index, &root, "src/Handler/WideHandler.php", "onAny"),
        [
            named("Sender.php", "dispatch"),
            named("Wide.php", "dispatch")
        ]
    );
}

const RECURRING_MESSAGE: &str = "<?php\nnamespace Symfony\\Component\\Scheduler;\nfinal class RecurringMessage\n{\n    public static function every(string $frequency, object $message): self { return new self(); }\n    public static function cron(string $expression, object $message): self { return new self(); }\n}\n";
const SCHEDULE: &str = "<?php\nnamespace App\\Service;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Scheduler\\RecurringMessage;\nfinal class Schedule\n{\n    public function messages(): array\n    {\n        return [\n            RecurringMessage::every('1 minute', new Ping()),\n            RecurringMessage::cron(message: new Ping(), expression: '* * * * *'),\n        ];\n    }\n}\n";

#[test]
fn recurring_messages_reach_their_handlers() {
    let (_dir, root) = messenger_symfony(&[
        ("lib/RecurringMessage.php", RECURRING_MESSAGE),
        ("src/Service/Schedule.php", SCHEDULE),
    ]);
    let mut index = auto(&root);

    for needle in ["every(", "cron("] {
        assert_eq!(
            handler_definitions(&mut index, &root, "src/Service/Schedule.php", needle),
            [named("PingHandler.php", "__invoke")],
            "{needle}"
        );
    }
    assert_eq!(
        handler_definitions(&mut index, &root, "src/Handler/PingHandler.php", "__invoke"),
        [
            named("Schedule.php", "cron"),
            named("Schedule.php", "every"),
            named("Sender.php", "dispatch")
        ]
    );
}

const TAGGED_SERVICES: &str = "parameters:\n    app.name: x\n\nservices:\n    _instanceof:\n        App\\Handler\\Handles:\n            tags: [messenger.message_handler]\n\n    App\\Handler\\TaggedHandler:\n        tags: [messenger.message_handler]\n\n    tagged.mapped:\n        class: App\\Handler\\MappedHandler\n        tags:\n            - { name: messenger.message_handler, handles: App\\Message\\Ping, method: onPing }\n\n    tagged.short:\n        class: App\\Handler\\ShortHandler\n        tags:\n            - messenger.message_handler: { method: run }\n\n    tagged.missing:\n        class: App\\Handler\\Nowhere\n        tags: [messenger.message_handler]\n\n    tagged.wrong:\n        class: App\\Handler\\MappedHandler\n        tags:\n            - { name: messenger.message_handler, method: nope }\n";

fn tagged_symfony() -> (TempDir, PathBuf) {
    messenger_symfony(&[
        ("config/services.yaml", TAGGED_SERVICES),
        (
            "src/Handler/TaggedHandler.php",
            "<?php\nnamespace App\\Handler;\nuse App\\Message\\Ping;\nfinal class TaggedHandler\n{\n    public function __invoke(Ping $message): void\n    {\n    }\n}\n",
        ),
        (
            "src/Handler/MappedHandler.php",
            "<?php\nnamespace App\\Handler;\nfinal class MappedHandler\n{\n    public function onPing(object $message): void\n    {\n    }\n}\n",
        ),
        (
            "src/Handler/ShortHandler.php",
            "<?php\nnamespace App\\Handler;\nuse App\\Message\\Ping;\nfinal class ShortHandler\n{\n    public function run(Ping $message): void\n    {\n    }\n}\n",
        ),
        (
            "src/Handler/Handles.php",
            "<?php\nnamespace App\\Handler;\ninterface Handles\n{\n}\n",
        ),
        (
            "src/Handler/InstanceHandler.php",
            "<?php\nnamespace App\\Handler;\nuse App\\Message\\Ping;\nfinal class InstanceHandler implements Handles\n{\n    public function __invoke(Ping $message): void\n    {\n    }\n}\n",
        ),
    ])
}

#[test]
fn tagged_handlers_handle_their_messages() {
    let (_dir, root) = tagged_symfony();
    let mut index = auto(&root);

    assert_eq!(
        handler_definitions(&mut index, &root, "src/Service/Sender.php", "dispatch("),
        [
            named("InstanceHandler.php", "__invoke"),
            named("MappedHandler.php", "onPing"),
            named("PingHandler.php", "__invoke"),
            named("ShortHandler.php", "run"),
            named("TaggedHandler.php", "__invoke")
        ]
    );
    for (file, method) in [
        ("src/Handler/TaggedHandler.php", "__invoke"),
        ("src/Handler/MappedHandler.php", "onPing"),
        ("src/Handler/ShortHandler.php", "run"),
        ("src/Handler/InstanceHandler.php", "__invoke"),
    ] {
        assert_eq!(
            handler_definitions(&mut index, &root, file, method),
            [named("Sender.php", "dispatch")],
            "{file}"
        );
    }
}

const BUSES: &str = "framework:\n    messenger:\n        default_bus: command.bus\n        buses:\n            command.bus: ~\n            query.bus: ~\n";

fn bus_handler(name: &str, bus: &str) -> String {
    format!(
        "<?php\nnamespace App\\Handler;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Messenger\\Attribute\\AsMessageHandler;\n#[AsMessageHandler(bus: '{bus}')]\nfinal class {name}\n{{\n    public function __invoke(Ping $message): void\n    {{\n    }}\n}}\n"
    )
}

fn bus_sender(name: &str, constructor: &str, receiver: &str) -> String {
    format!(
        "<?php\nnamespace App\\Service;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Messenger\\MessageBusInterface;\nuse Symfony\\Component\\DependencyInjection\\Attribute\\Target;\nfinal class {name}\n{{\n    private MessageBusInterface $bus;\n\n    public function __construct({constructor})\n    {{\n    }}\n\n    public function send(MessageBusInterface $queryBus): void\n    {{\n        {receiver}->dispatch(new Ping());\n    }}\n}}\n"
    )
}

fn bus_symfony() -> (TempDir, PathBuf) {
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    messenger_symfony(&[
        ("config/packages/messenger.yaml", BUSES),
        (
            "src/Handler/CommandHandler.php",
            leak(bus_handler("CommandHandler", "command.bus")),
        ),
        (
            "src/Handler/QueryHandler.php",
            leak(bus_handler("QueryHandler", "query.bus")),
        ),
        (
            "src/Service/QuerySender.php",
            leak(bus_sender(
                "QuerySender",
                "private MessageBusInterface $queryBus",
                "$this->queryBus",
            )),
        ),
        (
            "src/Service/ParameterSender.php",
            leak(bus_sender("ParameterSender", "", "$queryBus")),
        ),
        (
            "src/Service/AssignedSender.php",
            leak(bus_sender(
                "AssignedSender",
                "MessageBusInterface $queryBus",
                "$this->bus",
            )),
        ),
        (
            "src/Service/TargetSender.php",
            leak(bus_sender(
                "TargetSender",
                "#[Target('query.bus')] private MessageBusInterface $other",
                "$this->other",
            )),
        ),
    ])
}

#[test]
fn bus_handlers_list_only_dispatches_of_their_bus() {
    let (_dir, root) = bus_symfony();
    let mut index = auto(&root);
    let dispatch =
        |index: &mut Index, file: &str| handler_definitions(index, &root, file, "dispatch(");

    assert_eq!(
        dispatch(&mut index, "src/Service/QuerySender.php"),
        [
            named("PingHandler.php", "__invoke"),
            named("QueryHandler.php", "__invoke")
        ]
    );
    assert_eq!(
        dispatch(&mut index, "src/Service/ParameterSender.php"),
        [
            named("PingHandler.php", "__invoke"),
            named("QueryHandler.php", "__invoke")
        ]
    );
    // `$bus` names no bus: the default one.
    assert_eq!(
        dispatch(&mut index, "src/Service/Sender.php"),
        [
            named("CommandHandler.php", "__invoke"),
            named("PingHandler.php", "__invoke")
        ]
    );
    let query = handler_definitions(
        &mut index,
        &root,
        "src/Handler/QueryHandler.php",
        "__invoke",
    );
    assert!(
        query.iter().all(|(file, _)| file != "Sender.php"),
        "{query:?}"
    );
    assert!(
        query.contains(&named("QuerySender.php", "dispatch")),
        "{query:?}"
    );
}

#[test]
fn buses_the_module_cannot_tell_keep_every_handler() {
    let (_dir, root) = bus_symfony();
    let mut index = auto(&root);
    for file in [
        "src/Service/AssignedSender.php",
        "src/Service/TargetSender.php",
    ] {
        assert_eq!(
            handler_definitions(&mut index, &root, file, "dispatch("),
            [
                named("CommandHandler.php", "__invoke"),
                named("PingHandler.php", "__invoke"),
                named("QueryHandler.php", "__invoke")
            ],
            "{file}"
        );
    }
}

#[test]
fn handlers_without_bus_stay_listed() {
    let (_dir, root) = bus_symfony();
    let mut index = auto(&root);
    let all = handler_definitions(&mut index, &root, "src/Handler/PingHandler.php", "__invoke");
    for file in ["QuerySender.php", "Sender.php", "AssignedSender.php"] {
        assert!(all.contains(&named(file, "dispatch")), "{file} {all:?}");
    }
}

const QUERY_BUS: &str = "<?php\nnamespace App\\Bus;\nuse Symfony\\Component\\Messenger\\MessageBusInterface;\ninterface QueryBus extends MessageBusInterface\n{\n}\n";

fn sender_class(name: &str, body: &str) -> String {
    format!(
        "<?php\nnamespace App\\Service;\nuse App\\Bus\\QueryBus;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Messenger\\MessageBusInterface;\nfinal class {name}\n{{\n{body}\n}}\n"
    )
}

/// The handlers of the `dispatch(` in `sender`, on `bus_symfony` with `files`.
fn bus_case(files: &[(&str, String)], sender: &str) -> Vec<(String, String)> {
    let (_dir, root) = bus_symfony();
    for (file, content) in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let mut index = auto(&root);
    handler_definitions(&mut index, &root, sender, "dispatch(")
}

fn every_handler() -> Vec<(String, String)> {
    vec![
        named("CommandHandler.php", "__invoke"),
        named("PingHandler.php", "__invoke"),
        named("QueryHandler.php", "__invoke"),
    ]
}

#[test]
fn unread_messenger_configuration_keeps_every_handler() {
    let found = bus_case(
        &[(
            "config/packages/messenger.php",
            "<?php\nreturn static function (): void {};\n".to_string(),
        )],
        "src/Service/QuerySender.php",
    );
    assert_eq!(found, every_handler());
    let found = bus_case(
        &[(
            "config/packages/messenger.yaml",
            format!(
                "{BUSES}\nwhen@test:\n    framework:\n        messenger:\n            default_bus: query.bus\n"
            ),
        )],
        "src/Service/Sender.php",
    );
    assert_eq!(found, every_handler());
    let found = bus_case(
        &[(
            "config/packages/test/messenger.yaml",
            "framework:\n    messenger:\n        default_bus: query.bus\n".to_string(),
        )],
        "src/Service/Sender.php",
    );
    assert_eq!(found, every_handler());
}

#[test]
fn bus_constants_and_parsed_names_are_read() {
    let constant = bus_case(
        &[
            (
                "src/Bus/Buses.php",
                "<?php\nnamespace App\\Bus;\nfinal class Buses\n{\n    public const QUERY = 'query.bus';\n}\n".to_string(),
            ),
            (
                "src/Handler/QueryHandler.php",
                "<?php\nnamespace App\\Handler;\nuse App\\Bus\\Buses;\nuse App\\Message\\Ping;\nuse Symfony\\Component\\Messenger\\Attribute\\AsMessageHandler;\n#[AsMessageHandler(bus: Buses::QUERY)]\nfinal class QueryHandler\n{\n    public function __invoke(Ping $message): void\n    {\n    }\n}\n".to_string(),
            ),
        ],
        "src/Service/QuerySender.php",
    );
    assert_eq!(
        constant,
        [
            named("PingHandler.php", "__invoke"),
            named("QueryHandler.php", "__invoke")
        ]
    );
    let snake = bus_case(
        &[(
            "src/Service/SnakeSender.php",
            sender_class(
                "SnakeSender",
                "    public function __construct(private MessageBusInterface $query_bus)\n    {\n    }\n\n    public function send(): void\n    {\n        $this->query_bus->dispatch(new Ping());\n    }",
            ),
        )],
        "src/Service/SnakeSender.php",
    );
    assert_eq!(
        snake,
        [
            named("PingHandler.php", "__invoke"),
            named("QueryHandler.php", "__invoke")
        ]
    );
}

#[test]
fn buses_symfony_does_not_autowire_by_name_stay_unknown() {
    let cases = [
        (
            "HelperSender",
            "    public function __construct(private MessageBusInterface $queryBus)\n    {\n    }\n\n    public function run(): void\n    {\n        $this->send($this->queryBus);\n    }\n\n    private function send(MessageBusInterface $bus): void\n    {\n        $bus->dispatch(new Ping());\n    }",
        ),
        (
            "TypedSender",
            "    public function __construct(private QueryBus $bus)\n    {\n    }\n\n    public function send(): void\n    {\n        $this->bus->dispatch(new Ping());\n    }",
        ),
        (
            "SwappedSender",
            "    private MessageBusInterface $bus;\n\n    public function __construct(MessageBusInterface $bus, MessageBusInterface $queryBus)\n    {\n        $this->bus = $queryBus;\n    }\n\n    public function send(): void\n    {\n        $this->bus->dispatch(new Ping());\n    }",
        ),
    ];
    for (name, body) in cases {
        let file = format!("src/Service/{name}.php");
        let found = bus_case(
            &[
                ("src/Bus/QueryBus.php", QUERY_BUS.to_string()),
                (file.as_str(), sender_class(name, body)),
            ],
            &file,
        );
        assert_eq!(found, every_handler(), "{name}");
    }
    let bound = bus_case(
        &[(
            "config/services.yaml",
            "parameters:\n    app.name: x\n\nservices:\n    _defaults:\n        bind:\n            $bus: '@query.bus'\n".to_string(),
        )],
        "src/Service/Sender.php",
    );
    assert_eq!(bound, every_handler());
}

#[test]
fn prototype_tags_make_handlers() {
    let found = bus_case(
        &[
            (
                "config/services.yaml",
                "parameters:\n    app.name: x\n\nservices:\n    App\\Tagged\\:\n        resource: '../src/Tagged/'\n        tags: [messenger.message_handler]\n".to_string(),
            ),
            (
                "src/Tagged/PrototypeHandler.php",
                "<?php\nnamespace App\\Tagged;\nuse App\\Message\\Ping;\nfinal class PrototypeHandler\n{\n    public function __invoke(Ping $message): void\n    {\n    }\n}\n".to_string(),
            ),
        ],
        "src/Service/Sender.php",
    );
    assert!(
        found.contains(&named("PrototypeHandler.php", "__invoke")),
        "{found:?}"
    );
}

/// The `getParameter()` diagnostics of `broken_symfony` with `files`, whose
/// controller asks for `app.nowhere`.
fn parameter_messages(files: &[(&str, &str)]) -> Vec<String> {
    let (_dir, root) = broken_symfony(files);
    broken_messages(&root)
        .into_iter()
        .filter(|message| message.contains("parameter"))
        .collect()
}

const NOWHERE: &str = "Undefined parameter 'app.nowhere'";

#[test]
fn yml_and_environment_configuration_define_parameters() {
    for (file, content) in [
        (
            "config/packages/extra.yml",
            "parameters:\n    app.nowhere: 1\n",
        ),
        (
            "config/packages/dev/extra.yaml",
            "parameters:\n    app.nowhere: 1\n",
        ),
        (
            "config/services_dev.yml",
            "parameters:\n    app.nowhere: 1\n",
        ),
    ] {
        assert!(parameter_messages(&[(file, content)]).is_empty(), "{file}");
    }
    // Read files without the parameter keep it flagged.
    assert_eq!(
        parameter_messages(&[(
            "config/packages/dev/extra.yaml",
            "parameters:\n    app.other: 1\n"
        )]),
        [NOWHERE]
    );
}

#[test]
fn imported_files_define_parameters() {
    let files = [
        (
            "config/services.yaml",
            "imports:\n    - { resource: 'more/params.yaml' }\n\nparameters:\n    app.name: x\n",
        ),
        (
            "config/more/params.yaml",
            "parameters:\n    app.nowhere: 1\n",
        ),
    ];
    assert!(parameter_messages(&files).is_empty());
    let (_dir, root) = broken_symfony(&files);
    let mut index = auto(&root);
    let path = root.join("config/services.yaml");
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source).find("more/").unwrap() as u32 + 1;
    let found: Vec<PathBuf> = index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect();
    assert_eq!(found, [root.join("config/more/params.yaml")]);
}

#[test]
fn import_cycles_read_each_file_once() {
    let files = [
        (
            "config/services.yaml",
            "imports:\n    - { resource: 'a.yaml' }\n\nparameters:\n    app.name: x\n",
        ),
        (
            "config/a.yaml",
            "imports:\n    - { resource: 'services.yaml' }\n\nparameters:\n    app.other: 1\n",
        ),
    ];
    assert_eq!(parameter_messages(&files), [NOWHERE]);
}

#[test]
fn ignored_missing_imports_keep_names_certain() {
    let files = [(
        "config/services.yaml",
        "imports:\n    - { resource: 'gone.yaml', ignore_errors: true }\n\nparameters:\n    app.name: x\n",
    )];
    assert_eq!(parameter_messages(&files), [NOWHERE]);
}

#[test]
fn unreadable_imports_keep_names_unknown() {
    for import in ["'more/*.yaml'", "'gone.yaml'", "'legacy.xml'"] {
        let services =
            format!("imports:\n    - {{ resource: {import} }}\n\nparameters:\n    app.name: x\n");
        let services: &'static str = Box::leak(services.into_boxed_str());
        assert!(
            parameter_messages(&[("config/services.yaml", services)]).is_empty(),
            "{import}"
        );
    }
}

#[test]
fn files_the_kernel_does_not_load_define_nothing() {
    assert_eq!(
        parameter_messages(&[(
            "config/services.dist.yaml",
            "parameters:\n    app.nowhere: 1\n"
        )]),
        [NOWHERE]
    );
}

#[test]
fn parameters_set_from_php_lead_to_their_call() {
    let (_dir, root) = broken_symfony(&[
        (
            "lib/ContainerBuilder.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection;\nclass ContainerBuilder\n{\n    public function setParameter(string $name, mixed $value): void {}\n}\n",
        ),
        (
            "src/Pass.php",
            "<?php\nnamespace App;\nuse Symfony\\Component\\DependencyInjection\\ContainerBuilder;\nclass Pass\n{\n    public function process(ContainerBuilder $container): void\n    {\n        $container->setParameter('app.nowhere', 1);\n    }\n}\n",
        ),
        (
            "vendor/acme/lib/Other.php",
            "<?php\nnamespace Acme;\nuse Symfony\\Component\\DependencyInjection\\ContainerBuilder;\nclass Other\n{\n    public function process(ContainerBuilder $container): void\n    {\n        $container->setParameter('app.nowhere', 2);\n    }\n}\n",
        ),
    ]);
    let mut index = auto(&root);
    let path = root.join("src/Controller/Uses.php");
    let source = fs::read(&path).unwrap();
    let offset = String::from_utf8_lossy(&source)
        .find("app.nowhere")
        .unwrap() as u32
        + 1;
    let found: Vec<PathBuf> = index
        .definition(&path, &source, offset)
        .into_iter()
        .map(|location| location.path)
        .collect();

    assert_eq!(found, [root.join("src/Pass.php")]);
}
