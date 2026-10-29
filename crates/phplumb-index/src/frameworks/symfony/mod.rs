//! Symfony: the module that answers for a project built on the Symfony framework.

mod broken;
mod config;
mod doctrine;
mod files;
mod messenger;
mod routes;
mod templates;
mod twig;

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::diagnostics::Diagnostic;
use crate::filter::{is_doctrine_xml, is_yaml};
use crate::frameworks::ancestry::is_a;
use crate::frameworks::yaml::{self, Yaml};
use crate::frameworks::{
    AttributeString, CallTarget, Framework, FrameworkCall, FrameworkDeclaration, Project,
};
use crate::index::{Index, Location};
use crate::phpdoc::TypeExpr;
use crate::summary::{FileSummary, NameRef, RefKey, summarize};

use doctrine::Mappings;
use files::ConfigFiles;
use messenger::Handlers;
pub use routes::RouteTarget;
use routes::Routes;
pub use templates::TemplateStatus;
use templates::Templates;

/// Methods whose argument 0 is a route name, by the class that declares them.
const ROUTE_METHODS: [(&str, &[&str]); 2] = [
    (
        "Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController",
        &["redirectToRoute", "generateUrl"],
    ),
    (
        "Symfony\\Component\\Routing\\Generator\\UrlGeneratorInterface",
        &["generate"],
    ),
];

/// Methods whose argument 0 is a template name, by the class that declares them.
const TEMPLATE_METHODS: [(&str, &[&str]); 3] = [
    (
        "Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController",
        &[
            "render",
            "renderView",
            "renderBlock",
            "renderBlockView",
            "stream",
        ],
    ),
    ("Twig\\Environment", &["render", "display", "load"]),
    (
        "Symfony\\Bridge\\Twig\\Mime\\TemplatedEmail",
        &["htmlTemplate", "textTemplate"],
    ),
];

/// The method whose argument 0 is a parameter name.
const PARAMETER_METHODS: [(&str, &[&str]); 1] = [(
    "Symfony\\Bundle\\FrameworkBundle\\Controller\\AbstractController",
    &["getParameter"],
)];

/// Methods whose argument 0 is an entity class and that return its repository.
const REPOSITORY_METHODS: [(&str, &[&str]); 2] = [
    ("Doctrine\\Persistence\\ObjectManager", &["getRepository"]),
    ("Doctrine\\Persistence\\ManagerRegistry", &["getRepository"]),
];

/// The bus method whose argument 0 is a message.
const DISPATCH: [(&str, &[&str]); 1] = [(
    "Symfony\\Component\\Messenger\\MessageBusInterface",
    &["dispatch"],
)];

/// Classes whose `setParameter()` defines a container parameter.
const PARAMETER_HOLDERS: [&str; 4] = [
    "Symfony\\Component\\DependencyInjection\\Container",
    "Symfony\\Component\\DependencyInjection\\ContainerBuilder",
    "Symfony\\Component\\DependencyInjection\\ContainerInterface",
    "Symfony\\Component\\DependencyInjection\\ParameterBag\\ParameterBagInterface",
];

/// Classes through which PHP code configures the container or loads configuration
/// files, as kernels and bundle extensions do, so it may define any parameter.
const CONFIGURATORS: [&str; 8] = [
    "Symfony\\Component\\DependencyInjection\\Loader\\Configurator\\ContainerConfigurator",
    "Symfony\\Component\\Config\\Loader\\LoaderInterface",
    "Symfony\\Component\\DependencyInjection\\Loader\\YamlFileLoader",
    "Symfony\\Component\\DependencyInjection\\Loader\\XmlFileLoader",
    "Symfony\\Component\\DependencyInjection\\Loader\\PhpFileLoader",
    "Symfony\\Component\\DependencyInjection\\Extension\\ExtensionInterface",
    "Symfony\\Component\\DependencyInjection\\Extension\\Extension",
    "Symfony\\Component\\HttpKernel\\DependencyInjection\\Extension",
];

/// The attribute that injects a parameter, service or value into an argument.
const AUTOWIRE_ATTRIBUTE: &str = "Symfony\\Component\\DependencyInjection\\Attribute\\Autowire";

/// The attribute that names a controller method's template.
const TEMPLATE_ATTRIBUTE: &str = "Symfony\\Bridge\\Twig\\Attribute\\Template";

/// Whether `call` targets one of `methods`, by declaring class.
fn calls_one_of(index: &Index, call: &FrameworkCall<'_>, methods: &[(&str, &[&str])]) -> bool {
    call.targets.iter().any(|target| {
        let CallTarget::Method { class, name } = target else {
            return false;
        };
        methods.iter().any(|(owner, names)| {
            names.iter().any(|m| m.eq_ignore_ascii_case(name)) && is_a(index, class, &[owner])
        })
    })
}

/// The declarations of the class `fqn`, at their names.
fn class_targets(index: &Index, fqn: &str) -> Vec<(Location, String)> {
    let fqn = fqn.trim_start_matches('\\');
    let short = fqn.rsplit('\\').next().unwrap_or(fqn).to_string();
    index
        .classes_named(fqn)
        .iter()
        .map(|class| {
            (
                Location {
                    path: class.path.to_path_buf(),
                    start: class.declaration.name_start,
                    end: class.declaration.name_end,
                },
                short.clone(),
            )
        })
        .collect()
}

/// The declaration of property `name` on `class` or its parents, at its name.
fn property_targets(index: &Index, class: &str, name: &str) -> Vec<(Location, String)> {
    index
        .find_member(class, crate::summary::MemberKind::Property, name)
        .into_iter()
        .map(|(found, member)| {
            (
                Location {
                    path: found.path.to_path_buf(),
                    start: member.name_start,
                    end: member.name_end,
                },
                member.name.clone(),
            )
        })
        .collect()
}

/// Targets as `(location, expected text)` pairs.
fn targets_of(targets: &[RouteTarget]) -> Vec<(Location, String)> {
    targets
        .iter()
        .map(|target| (target.location.clone(), target.text.clone()))
        .collect()
}

/// A file target at its start.
fn file_start(path: PathBuf) -> (Location, String) {
    (
        Location {
            path,
            start: 0,
            end: 0,
        },
        String::new(),
    )
}

#[derive(Default)]
pub struct Symfony {
    project: Project,
    routes: Routes,
    templates: Templates,
    handlers: Handlers,
    /// Parameter definitions, by name.
    parameters: HashMap<String, Vec<RouteTarget>>,
    /// Whether configuration the module does not read may define parameters.
    parameters_unknown: bool,
    /// The parameters project code sets with `setParameter()`, or none when one of
    /// those calls computes its name; found on first use after each read.
    set_parameters: OnceLock<Option<HashSet<String>>>,
    /// Service definitions, by id.
    services: HashMap<String, Vec<RouteTarget>>,
    mappings: Mappings,
    /// The project's `*.orm.xml` files, listed again only when a directory or a
    /// mapping file changes.
    mapping_files: Vec<PathBuf>,
    /// The YAML and XML files read, kept until they change.
    config_files: ConfigFiles,
}

impl Symfony {
    /// Where the route `name` is defined: an attribute route's controller method, or
    /// a YAML route's key and controller method.
    pub fn route_targets(&self, name: &str) -> &[RouteTarget] {
        self.routes.targets(name)
    }

    /// The file the template `name` resolves to, as Twig's loader finds it.
    pub fn template_file(&self, name: &str) -> Option<PathBuf> {
        self.templates.file(name)
    }

    /// Whether the template `name` resolves to a file, is missing from every
    /// directory of its namespace, or cannot be known.
    pub fn template_status(&self, name: &str) -> TemplateStatus {
        self.templates.status(name)
    }

    /// Whether no route is named `name`, with every source of route names read.
    pub fn route_missing(&self, name: &str) -> bool {
        self.routes.missing(name)
    }

    /// Whether no parameter is named `name`, with every source of parameters read.
    /// Bundles define their own parameters, so only a name whose first segment
    /// starts a parameter of the project counts; `kernel.*` and `env(...)` never do.
    pub fn parameter_missing(&self, index: &Index, name: &str) -> bool {
        if self.parameters_unknown
            || self.parameters.contains_key(name)
            || name.starts_with("kernel.")
            || name.starts_with("env(")
        {
            return false;
        }
        let family = |name: &str| name.split('.').next().unwrap_or_default().to_string();
        let ours = family(name);
        if !self
            .parameters
            .keys()
            .any(|defined| family(defined) == ours)
        {
            return false;
        }
        self.set_parameters(index)
            .is_some_and(|set| !set.contains(name))
    }

    /// Whether `path` is code of the project, not a dependency nor what Symfony
    /// generates in `var`, such as the compiled container.
    fn is_source(&self, index: &Index, path: &Path) -> bool {
        self.project.contains(path)
            && index.is_project_code(path)
            && !path.starts_with(self.project.dir.join("var"))
    }

    /// The literal names `setParameter()` calls and parameter bag `set()` calls in
    /// project code give the container, or none when one of them is computed. Only
    /// files that mention a container class are read: a `setParameter()` whose
    /// receiver stays unknown there counts.
    fn set_parameters(&self, index: &Index) -> Option<&HashSet<String>> {
        self.set_parameters
            .get_or_init(|| {
                let holders: Vec<&Path> = PARAMETER_HOLDERS
                    .iter()
                    .flat_map(|class| index.candidate_files(&RefKey::Class(class.to_string())))
                    .collect();
                let mut files: Vec<&Path> = ["setParameter", "getParameterBag"]
                    .iter()
                    .flat_map(|name| index.candidate_files(&RefKey::Member(name.to_string())))
                    .filter(|path| holders.contains(path))
                    .filter(|path| self.is_source(index, path))
                    .collect();
                files.sort();
                files.dedup();
                let mut names = HashSet::new();
                for path in files {
                    let source = std::fs::read(path).ok()?;
                    let summary = summarize(&source);
                    let calls = crate::infer::calls_named(
                        index,
                        path,
                        &summary,
                        &source,
                        &["setParameter", "set"],
                    );
                    for call in calls {
                        let (start, end) = (call.location.start, call.location.end);
                        let method = String::from_utf8_lossy(&source[start as usize..end as usize]);
                        let holder = call.targets.iter().any(|target| {
                            matches!(target, CallTarget::Method { class, .. }
                                if is_a(index, class, &PARAMETER_HOLDERS))
                        });
                        let unknown =
                            call.targets.is_empty() && method.eq_ignore_ascii_case("setParameter");
                        if holder || unknown {
                            names.insert(call.strings.first().cloned().flatten()?);
                        }
                    }
                }
                Some(names)
            })
            .as_ref()
    }

    /// The handler methods of the message class `message`, at their names.
    pub fn handlers_of(&self, index: &Index, message: &str) -> Vec<Location> {
        self.handlers
            .of(index, message)
            .into_iter()
            .map(|(location, _)| location)
            .collect()
    }

    /// The message classes the handler method named at `start` in `path` handles.
    pub fn handled_by(&self, path: &Path, start: u32) -> Vec<String> {
        self.handlers.handled_by(path, start)
    }

    /// The `dispatch()` calls of the project whose message is, extends or
    /// implements one of `handled`, at their method name.
    fn dispatches(&self, index: &Index, handled: &[String]) -> Vec<(Location, String)> {
        let handled: Vec<&str> = handled.iter().map(String::as_str).collect();
        index
            .calls_named(&["dispatch"], &self.project)
            .into_iter()
            .filter(|call| {
                let found = FrameworkCall {
                    targets: &call.targets,
                    strings: &call.strings,
                    receiver: Some(&call.receiver),
                };
                calls_one_of(index, &found, &DISPATCH)
                    && call.arguments.first().is_some_and(|message| {
                        crate::infer::class_names(message)
                            .iter()
                            .any(|class| is_a(index, class, &handled))
                    })
            })
            .map(|call| (call.location, "dispatch".to_string()))
            .collect()
    }

    /// Where the parameter `name` is defined under `parameters:`.
    pub fn parameter_targets(&self, name: &str) -> &[RouteTarget] {
        self.parameters.get(name).map_or(&[], Vec::as_slice)
    }

    /// The definition of the service `id`, or the class when the id names one that no
    /// key defines.
    pub fn service_targets(&self, index: &Index, id: &str) -> Vec<(Location, String)> {
        match self.services.get(id) {
            Some(targets) => targets_of(targets),
            None if config::looks_like_class(id) => class_targets(index, id),
            None => Vec::new(),
        }
    }

    /// In a YAML configuration file: a `%parameter%`, an `@id` service reference, or a
    /// class name under `offset`.
    fn yaml_targets(
        &self,
        index: &Index,
        path: &Path,
        source: &[u8],
        offset: u32,
    ) -> Vec<(Location, String)> {
        // Elsewhere, such as in translations, `%name%` is a placeholder.
        let config = self.project.dir.join("config");
        if !index.filter().under_root(path).starts_with(&config) {
            return Vec::new();
        }
        let Ok(text) = std::str::from_utf8(source) else {
            return Vec::new();
        };
        let Some(root) = yaml::parse(text) else {
            return Vec::new();
        };
        let Some((steps, Yaml::Scalar { value, start, end })) = yaml::scalar_at(&root, offset)
        else {
            return Vec::new();
        };
        let in_services = steps.contains(&yaml::Step::Key("services"));
        let names_a_service = matches!(
            steps.last(),
            Some(yaml::Step::Key("alias" | "decorates" | "parent"))
        );
        if in_services && names_a_service {
            return self.service_targets(index, value);
        }
        let raw = &text[*start as usize..*end as usize];
        if let Some(name) = config::parameter_at(raw, (offset - start) as usize) {
            return targets_of(self.parameter_targets(&name));
        }
        if let Some(reference) = value.strip_prefix('@') {
            // `@?id` is optional, `@=` an expression and `@@` an escaped `@`.
            if reference.starts_with('=') || reference.starts_with('@') {
                return Vec::new();
            }
            return self.service_targets(index, reference.trim_start_matches('?'));
        }
        if config::looks_like_class(value) {
            return class_targets(index, value);
        }
        Vec::new()
    }

    /// The repository class the Doctrine mappings declare for `entity`.
    pub fn repository_of(&self, entity: &str) -> Option<&str> {
        self.mappings.repository_of(entity)
    }

    /// The `%name%` under the cursor in the value of an attribute.
    fn value_parameter_targets(
        &self,
        found: &AttributeString,
        offset: u32,
    ) -> Vec<(Location, String)> {
        config::parameter_at(&found.value, offset.saturating_sub(found.start) as usize)
            .map(|name| targets_of(self.parameter_targets(&name)))
            .unwrap_or_default()
    }

    fn read(&mut self, index: &Index) {
        let filter = index.filter().scoped(&self.project);
        let files = config::config_files(&self.project, &filter);
        self.parameters = config::read_parameters(&files, &self.config_files);
        let configured_in_php = CONFIGURATORS
            .iter()
            .flat_map(|class| index.candidate_files(&RefKey::Class(class.to_string())))
            .any(|path| self.is_source(index, path));
        self.parameters_unknown = configured_in_php
            || config::has_unread_configuration(&self.project, &files, &self.config_files);
        self.set_parameters = OnceLock::new();
        self.services = config::read_services(&files, &self.config_files);
        self.mappings = Mappings::read(index, &filter, &self.mapping_files, &self.config_files);
        self.routes = Routes::read(index, &filter, &self.project, &self.config_files, &files);
        self.templates = Templates::read(&self.project, &self.config_files);
        self.handlers = Handlers::read(index, &filter);
    }
}

impl Framework for Symfony {
    fn name(&self) -> &'static str {
        "symfony"
    }

    fn package(&self) -> &'static str {
        "symfony/framework-bundle"
    }

    fn project(&self) -> &Project {
        &self.project
    }

    fn set_project(&mut self, project: Project) {
        self.project = project;
    }

    fn dependency_directories(&self) -> &'static [&'static str] {
        &[]
    }

    fn php_extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn rebuild(&mut self, index: &Index) {
        let filter = index.filter().scoped(&self.project);
        self.mapping_files = doctrine::mapping_files(index, &self.project.dir, &filter);
        self.read(index);
    }

    /// Reads the routes, template paths, message handlers, configuration and Doctrine
    /// mappings again when a PHP, YAML or XML file of the project, or a directory,
    /// changed.
    fn update(&mut self, index: &Index, changed: &[PathBuf], removed: &[PathBuf]) {
        let paths: Vec<PathBuf> = changed
            .iter()
            .chain(removed)
            .map(|path| index.filter().under_root(path))
            .filter(|path| self.project.contains(path) || self.project.dir.starts_with(path))
            .collect();
        let relevant = paths.iter().any(|path| {
            let kind = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            path.is_dir() || matches!(kind.as_deref(), None | Some("php" | "yaml" | "yml" | "xml"))
        });
        let configuration: HashSet<PathBuf> = paths
            .iter()
            .filter(|path| {
                path.is_dir()
                    || path.extension().is_none()
                    || is_yaml(path)
                    || is_doctrine_xml(path)
            })
            .flat_map(|path| [Some(path.clone()), path.canonicalize().ok()])
            .flatten()
            .collect();
        self.config_files.forget(&configuration);
        let mappings_moved = paths
            .iter()
            .any(|path| path.is_dir() || path.extension().is_none() || is_doctrine_xml(path));
        if mappings_moved {
            let filter = index.filter().scoped(&self.project);
            self.mapping_files = doctrine::mapping_files(index, &self.project.dir, &filter);
        }
        if relevant {
            self.read(index);
        }
    }

    fn typed_methods(&self) -> &'static [&'static str] {
        &["getrepository"]
    }

    fn targeted_methods(&self) -> &'static [&'static str] {
        &["dispatch"]
    }

    /// `getRepository(X::class)` returns the repository the mappings declare for `X`,
    /// when the index knows that class.
    fn call_type(&self, index: &Index, call: &FrameworkCall<'_>) -> Option<TypeExpr> {
        if !calls_one_of(index, call, &REPOSITORY_METHODS) {
            return None;
        }
        let entity = call.strings.first()?.as_deref()?;
        let repository = self.repository_of(entity)?;
        index
            .has_class(repository)
            .then(|| TypeExpr::Named(repository.to_string()))
    }

    /// A template name given to a rendering method opens the template; a route name
    /// given to a route method leads to the route's definitions.
    fn string_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        argument: usize,
        value: &str,
    ) -> Vec<(Location, String)> {
        if argument != 0 {
            return Vec::new();
        }
        if calls_one_of(index, call, &PARAMETER_METHODS) {
            return targets_of(self.parameter_targets(value));
        }
        if calls_one_of(index, call, &TEMPLATE_METHODS) {
            return self
                .template_file(value)
                .into_iter()
                .map(file_start)
                .collect();
        }
        if !calls_one_of(index, call, &ROUTE_METHODS) {
            return Vec::new();
        }
        self.route_targets(value)
            .iter()
            .map(|target| (target.location.clone(), target.text.clone()))
            .collect()
    }

    /// A parameter or service in `#[Autowire]` leads to its definitions; a template
    /// in `#[Template]` opens its file.
    fn attribute_targets(
        &self,
        index: &Index,
        found: &AttributeString,
        offset: u32,
    ) -> Vec<(Location, String)> {
        let first = found.key.is_none() && found.position == 0;
        if found.attribute.eq_ignore_ascii_case(AUTOWIRE_ATTRIBUTE) {
            return match found.key.as_deref() {
                Some("param") => targets_of(self.parameter_targets(&found.value)),
                Some("service") => self.service_targets(index, &found.value),
                Some("value") => self.value_parameter_targets(found, offset),
                None if first => self.value_parameter_targets(found, offset),
                _ => Vec::new(),
            };
        }
        let template = found.key.as_deref() == Some("template") || first;
        if found.attribute.eq_ignore_ascii_case(TEMPLATE_ATTRIBUTE) && template {
            return self
                .template_file(&found.value)
                .into_iter()
                .map(file_start)
                .collect();
        }
        Vec::new()
    }

    /// A `dispatch()` on a message bus lists the handlers of its message.
    fn call_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        arguments: &[TypeExpr],
    ) -> Vec<(Location, String)> {
        if !calls_one_of(index, call, &DISPATCH) {
            return Vec::new();
        }
        let Some(message) = arguments.first() else {
            return Vec::new();
        };
        let mut found: Vec<(Location, String)> = Vec::new();
        for class in crate::infer::class_names(message) {
            for (location, name) in self.handlers.of(index, &class) {
                if !found.iter().any(|(known, _)| *known == location) {
                    found.push((location, name));
                }
            }
        }
        found
    }

    fn keyed_string_targets(
        &self,
        _index: &Index,
        _key: &str,
        _value: &str,
        _call: Option<(&FrameworkCall<'_>, usize)>,
    ) -> Vec<(Location, String)> {
        Vec::new()
    }

    /// Route, template and parameter names that are defined nowhere, in PHP, Twig
    /// templates and YAML configuration.
    fn diagnostics(
        &self,
        index: &Index,
        path: &Path,
        source: &[u8],
        summary: Option<&FileSummary>,
    ) -> Vec<Diagnostic> {
        if crate::filter::is_twig(path) {
            return broken::find_in_twig(self, index, path, source);
        }
        if crate::filter::is_yaml(path) {
            return broken::find_in_yaml(self, index, path, source);
        }
        match summary {
            Some(summary) => broken::find(self, index, path, source, summary),
            None => Vec::new(),
        }
    }

    fn class_implementations(&self, _index: &Index, _class: &str) -> Vec<(Location, String)> {
        Vec::new()
    }

    fn annotation_classes(
        &self,
        _index: &Index,
        _class: &str,
        _name: &NameRef,
    ) -> Option<Vec<String>> {
        None
    }

    /// In YAML configuration, a parameter, a service reference or a class name leads
    /// to its definition; in a Doctrine mapping, a class or property name opens it;
    /// in a Twig template, a template name opens its file, and a route name in
    /// `path()` or `url()` leads to the route.
    fn file_targets(
        &self,
        index: &Index,
        path: &Path,
        source: &[u8],
        offset: u32,
    ) -> Vec<(Location, String)> {
        if crate::filter::is_yaml(path) {
            return self.yaml_targets(index, path, source, offset);
        }
        if crate::filter::is_doctrine_xml(path) {
            let Ok(text) = std::str::from_utf8(source) else {
                return Vec::new();
            };
            return match doctrine::named_at(text, offset) {
                Some(doctrine::Named::Class(class)) => class_targets(index, &class),
                Some(doctrine::Named::Property(class, property)) => {
                    property_targets(index, &class, &property)
                }
                None => Vec::new(),
            };
        }
        if !crate::filter::is_twig(path) {
            return Vec::new();
        }
        if let Some((name, _, _)) = twig::template_name_at(source, offset) {
            return self
                .template_file(&name)
                .into_iter()
                .map(file_start)
                .collect();
        }
        let Some((name, _, _)) = twig::route_name_at(source, offset) else {
            return Vec::new();
        };
        self.route_targets(&name)
            .iter()
            .map(|target| (target.location.clone(), target.text.clone()))
            .collect()
    }

    /// A message handler, by its method or its `#[AsMessageHandler]`, lists the
    /// dispatches of its messages.
    fn declaration_targets(
        &self,
        index: &Index,
        declaration: &FrameworkDeclaration<'_>,
    ) -> Vec<(Location, String)> {
        let handled = match declaration {
            FrameworkDeclaration::Method { path, member, .. } => {
                self.handled_by(path, member.name_start)
            }
            FrameworkDeclaration::Class {
                path,
                class,
                metadata,
            } => messenger::handler_method(class, metadata)
                .map(|method| self.handled_by(path, method.name_start))
                .unwrap_or_default(),
            FrameworkDeclaration::Function { .. } => Vec::new(),
        };
        if handled.is_empty() {
            return Vec::new();
        }
        self.dispatches(index, &handled)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
