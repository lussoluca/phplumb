//! Symfony: the module that answers for a project built on the Symfony framework.

mod broken;
mod bundles;
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
    AttributeString, CallArguments, CallTarget, Framework, FrameworkCall, FrameworkDeclaration,
    Project, ReceiverName,
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

/// The methods that send a message to its handlers, by declaring class, with the
/// position of the message: a bus's `dispatch()`, `HandleTrait::handle()`, and the
/// Scheduler's recurring messages.
const MESSAGE_CALLS: [(&str, &str, usize); 5] = [
    (
        "Symfony\\Component\\Messenger\\MessageBusInterface",
        "dispatch",
        0,
    ),
    ("Symfony\\Component\\Messenger\\HandleTrait", "handle", 0),
    (
        "Symfony\\Component\\Scheduler\\RecurringMessage",
        "every",
        1,
    ),
    ("Symfony\\Component\\Scheduler\\RecurringMessage", "cron", 1),
    (
        "Symfony\\Component\\Scheduler\\RecurringMessage",
        "trigger",
        1,
    ),
];

/// The interface message buses are autowired by.
const MESSAGE_BUS: &str = "Symfony\\Component\\Messenger\\MessageBusInterface";

/// The name of the argument Symfony autowires the bus `bus` to:
/// `registerAliasForArgument()` turns `command.bus` into `commandBus`.
fn bus_alias(bus: &str) -> String {
    let mut alias = String::new();
    let mut upper = false;
    for c in bus.chars() {
        if c.is_ascii_alphanumeric() {
            alias.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else {
            upper = !alias.is_empty();
        }
    }
    let mut chars = alias.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_ascii_lowercase().to_string() + chars.as_str()
    })
}

/// Whether the parameter `$name` of the function declared by `signature` has an
/// attribute, which may name another service; true when the parameter is not
/// found.
fn parameter_has_attribute(signature: &str, name: &str) -> bool {
    let variable = format!("${name}");
    let bytes = signature.as_bytes();
    let Some(at) = signature
        .match_indices(&variable)
        .map(|(at, _)| at)
        .find(|&at| {
            let end = at + variable.len();
            bytes
                .get(end)
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
        })
    else {
        return true;
    };
    // Back to the comma or parenthesis that starts the parameter.
    let mut depth = 0;
    let mut start = at;
    while start > 0 {
        start -= 1;
        match bytes[start] {
            b')' | b']' => depth += 1,
            b'(' | b'[' if depth == 0 => break,
            b'(' | b'[' => depth -= 1,
            b',' if depth == 0 => break,
            _ => {}
        }
    }
    signature[start..at].contains("#[")
}

/// Lowercased names of the `MESSAGE_CALLS` methods.
const MESSAGE_METHODS: [&str; 5] = ["dispatch", "handle", "every", "cron", "trigger"];

/// The position of the message of `call`, when it is one of `MESSAGE_CALLS`.
fn message_position(index: &Index, call: &FrameworkCall<'_>) -> Option<usize> {
    call.targets.iter().find_map(|target| {
        let CallTarget::Method { class, name } = target else {
            return None;
        };
        MESSAGE_CALLS
            .iter()
            .find(|(owner, method, _)| {
                method.eq_ignore_ascii_case(name) && is_a(index, class, &[owner])
            })
            .map(|(_, _, position)| *position)
    })
}

/// The envelope a message may be dispatched in.
const ENVELOPE: &str = "Symfony\\Component\\Messenger\\Envelope";

/// The message classes the argument at `position` holds: the type of the message
/// an `Envelope` built in place wraps.
fn messages(
    index: &Index,
    arguments: &[TypeExpr],
    wrapped: &[Option<TypeExpr>],
    position: usize,
) -> Vec<String> {
    let Some(message) = arguments.get(position) else {
        return Vec::new();
    };
    let classes = crate::infer::class_names(message);
    let enveloped = classes.iter().any(|class| is_a(index, class, &[ENVELOPE]));
    match wrapped.get(position) {
        Some(Some(inner)) if enveloped => crate::infer::class_names(inner),
        _ => classes,
    }
}

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
    /// The message buses and the default one; none when they cannot be told.
    buses: Option<(Vec<String>, String)>,
    /// Each service alias and the service it stands for.
    aliases: HashMap<String, String>,
    /// The argument names the configuration binds, and the classes whose
    /// arguments it wires.
    bindings: (Vec<String>, Vec<String>),
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
            .of(index, message, None)
            .into_iter()
            .map(|(location, _)| location)
            .collect()
    }

    /// The message classes the handler method named at `start` in `path` handles.
    pub fn handled_by(&self, path: &Path, start: u32) -> Vec<String> {
        self.handlers.handled_by(path, start)
    }

    /// The bus a call made on `receiver` from `caller` goes through, when the
    /// module can tell: a promoted constructor property typed `MessageBusInterface`
    /// without attributes, which Symfony autowires by name to the bus with that
    /// alias or else to the default bus, or a parameter of the calling method named
    /// like a bus alias; none when the configuration binds the name or wires the
    /// class's arguments.
    fn bus_of(
        &self,
        index: &Index,
        receiver: Option<&ReceiverName>,
        caller: Option<&str>,
    ) -> Option<String> {
        let (buses, default) = self.buses.as_ref()?;
        let (class, method) = caller?.split_once("::")?;
        let (function, name, property) = match receiver? {
            ReceiverName::Property(name) => ("__construct", name, true),
            ReceiverName::Variable(name) => (method, name, false),
        };
        let (bound, wired) = &self.bindings;
        if bound.iter().any(|key| key.ends_with(&format!("${name}")))
            || wired.iter().any(|wired| wired.eq_ignore_ascii_case(class))
        {
            return None;
        }
        let (found, member) = index
            .find_member(class, crate::summary::MemberKind::Method, function)
            .into_iter()
            .next()?;
        let position = member.params.iter().position(|(param, _)| param == name)?;
        let typed = member
            .hints
            .get(position)?
            .as_ref()
            .is_some_and(|hint| crate::infer::class_names(hint) == [MESSAGE_BUS]);
        if !typed {
            return None;
        }
        if property {
            // Promoted: the property is declared by the constructor parameter.
            let (_, declared) = index
                .find_member(class, crate::summary::MemberKind::Property, name)
                .into_iter()
                .next()?;
            if declared.range.0 < member.range.0 || declared.range.1 > member.range.1 {
                return None;
            }
        }
        // The index keeps no signatures: the declaration is read from its file.
        let text = std::fs::read(found.path).ok()?;
        let (start, end) = member.range;
        let declaration = String::from_utf8_lossy(text.get(start as usize..end as usize)?);
        if parameter_has_attribute(&declaration, name) {
            return None;
        }
        let alias = bus_alias(name);
        match buses.iter().find(|bus| bus_alias(bus) == alias) {
            Some(bus) => Some(bus.clone()),
            None if property => Some(default.clone()),
            None => None,
        }
    }

    /// The calls of the project that send a message, `MESSAGE_CALLS`, whose message
    /// may be one of `handled`, at their method name, on one of `buses` (none for
    /// every bus) when the module can tell a call's bus.
    fn dispatches(
        &self,
        index: &Index,
        handled: &[String],
        buses: &[Option<String>],
    ) -> Vec<(Location, String)> {
        let handled: Vec<&str> = handled.iter().map(String::as_str).collect();
        index
            .calls_named(&MESSAGE_METHODS, &self.project)
            .into_iter()
            .filter(|call| {
                let found = FrameworkCall {
                    targets: &call.targets,
                    strings: &call.strings,
                    receiver: Some(&call.receiver),
                };
                // The bus is read only when a handler here is limited to one.
                let on_bus = || {
                    buses.iter().all(Option::is_none)
                        || self
                            .bus_of(index, call.receiver_name.as_ref(), call.caller.as_deref())
                            .is_none_or(|bus| {
                                buses
                                    .iter()
                                    .any(|own| own.as_ref().is_none_or(|own| *own == bus))
                            })
                };
                message_position(index, &found).is_some_and(|position| {
                    messages(index, &call.arguments, &call.wrapped, position)
                        .iter()
                        .any(|message| {
                            handled
                                .iter()
                                .any(|class| messenger::may_carry(index, message, class))
                        })
                }) && on_bus()
            })
            .map(|call| {
                let name = call.targets.iter().find_map(|target| match target {
                    CallTarget::Method { name, .. } => Some(name.clone()),
                    CallTarget::Function(_) => None,
                });
                (
                    call.location,
                    name.unwrap_or_else(|| "dispatch".to_string()),
                )
            })
            .collect()
    }

    /// Where the parameter `name` is defined under `parameters:`.
    pub fn parameter_targets(&self, name: &str) -> &[RouteTarget] {
        self.parameters.get(name).map_or(&[], Vec::as_slice)
    }

    /// The definition of the service `id`, or the class when the id names one that no
    /// key defines; for an alias, then those of the service it stands for.
    pub fn service_targets(&self, index: &Index, id: &str) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut next = Some(id);
        // An alias also leads to the service it stands for, through a chain of them.
        while let Some(id) = next.take() {
            if seen.contains(&id) {
                break;
            }
            seen.push(id);
            let targets = match self.services.get(id) {
                Some(targets) => targets_of(targets),
                None if config::looks_like_class(id) => class_targets(index, id),
                None => Vec::new(),
            };
            for target in targets {
                if !found.contains(&target) {
                    found.push(target);
                }
            }
            next = self.aliases.get(id).map(String::as_str);
        }
        found
    }

    /// In a YAML configuration file: a `%parameter%`, a template name, an `@id`
    /// service reference, or a class name under `offset`.
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
        // A template name, such as a `form_themes` entry; with `@`, a namespaced one.
        let namespaced = !value.starts_with('@') || value.contains('/');
        if namespaced && value.to_ascii_lowercase().ends_with(".twig") {
            return self
                .template_file(value)
                .into_iter()
                .map(file_start)
                .collect();
        }
        if value.starts_with('@') {
            // `@=` is an expression and `@@` an escaped `@`.
            return config::service_reference(value)
                .map(|id| self.service_targets(index, id))
                .unwrap_or_default();
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
        self.aliases = config::read_aliases(&files, &self.config_files);
        self.mappings = Mappings::read(index, &filter, &self.mapping_files, &self.config_files);
        self.routes = Routes::read(index, &filter, &self.project, &self.config_files, &files);
        self.templates = Templates::read(index, &self.project, &self.config_files);
        let tagged =
            config::tagged_services(&files, &self.config_files, "messenger.message_handler");
        self.handlers = Handlers::read(index, &filter, &tagged);
        // Handlers limited to a bus the configuration does not define leave the buses
        // unknown, as does configuration the module does not read.
        self.buses = config::buses(&files, &self.config_files)
            .filter(|_| !self.parameters_unknown)
            .filter(|(names, _)| {
                self.handlers
                    .buses()
                    .all(|bus| names.iter().any(|name| name == bus))
            });
        self.bindings = config::bindings(&files, &self.config_files);
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
        &MESSAGE_METHODS
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
            // `__construct($value, $service, $expression, $env, $param)`.
            let position = match found.key.as_deref() {
                Some("value") => Some(0),
                Some("service") => Some(1),
                Some("param") => Some(4),
                Some(_) => None,
                None => Some(found.position),
            };
            return match position {
                // A value `@id` names a service, `@@` escapes an `@`.
                Some(0) if found.value.starts_with('@') => config::service_reference(&found.value)
                    .map(|id| self.service_targets(index, id))
                    .unwrap_or_default(),
                Some(0) => self.value_parameter_targets(found, offset),
                Some(1) => self.service_targets(index, &found.value),
                Some(4) => targets_of(self.parameter_targets(&found.value)),
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

    /// A call that sends a message, such as `dispatch()` on a message bus, lists the
    /// handlers of its message.
    fn call_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        arguments: &CallArguments<'_>,
    ) -> Vec<(Location, String)> {
        let Some(position) = message_position(index, call) else {
            return Vec::new();
        };
        let bus = match self.handlers.buses().next() {
            Some(_) => self.bus_of(index, arguments.receiver, arguments.caller),
            None => None,
        };
        let mut found: Vec<(Location, String)> = Vec::new();
        for class in messages(index, arguments.types, arguments.wrapped, position) {
            for (location, name) in self.handlers.of(index, &class, bus.as_deref()) {
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

    /// In YAML configuration, a parameter, a template name, a service reference or a
    /// class name leads to its definition; in a Doctrine mapping, a class or
    /// property name opens it; in a Twig template, a template name opens its file,
    /// and a route name in `path()` or `url()` leads to the route.
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
        // Handlers are stored at the paths the index holds, whatever path the editor
        // opened the root through.
        let start = match declaration {
            FrameworkDeclaration::Method { path, member, .. } => {
                Some((index.filter().under_root(path), member.name_start))
            }
            FrameworkDeclaration::Class {
                path,
                class,
                metadata,
            } => messenger::handler_method(class, metadata)
                .map(|method| (index.filter().under_root(path), method.name_start)),
            FrameworkDeclaration::Function { .. } => None,
        };
        let Some((path, start)) = start else {
            return Vec::new();
        };
        let handled = self.handled_by(&path, start);
        if handled.is_empty() {
            return Vec::new();
        }
        self.dispatches(index, &handled, &self.handlers.buses_of(&path, start))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
