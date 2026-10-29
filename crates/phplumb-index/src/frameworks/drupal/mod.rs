//! Drupal: services, entity types, hooks, plugins and routes.

mod broken;
mod entities;
mod extensions;
mod hooks;
mod literals;
mod navigation;
mod plugins;
mod routes;
mod services;

use std::any::Any;
use std::path::{Path, PathBuf};

use crate::diagnostics::Diagnostic;
use crate::frameworks::ancestry::is_a;
use crate::frameworks::{CallTarget, Framework, FrameworkCall, FrameworkDeclaration, Project};
use crate::index::{Index, Location};
use crate::phpdoc::TypeExpr;
use crate::summary::NameRef;

pub use entities::EntityType;
use entities::EntityTypes;
pub use hooks::{ExtensionKind, HookDoc, HookImplementation};
use hooks::{Hooks, Storage};
use literals::PhpLiterals;

use plugins::Plugins;
pub use plugins::{Plugin, PluginType};
pub use routes::RouteDefinition;
use routes::Routes;
pub use services::ServiceDefinition;
use services::Services;

#[derive(Default)]
pub struct Drupal {
    services: Services,
    entities: EntityTypes,
    hooks: Hooks,
    plugins: Plugins,
    routes: Routes,
    /// What PHP code may define as ids.
    literals: PhpLiterals,
    project: Project,
}

/// Services the kernel sets itself.
const SYNTHETIC: [&str; 3] = ["service_container", "kernel", "class_loader"];

impl Drupal {
    /// Whether `id` is an entity type stored as `storage` asks.
    fn has_entity_type(&self, index: &Index, id: &str, storage: Storage) -> bool {
        let config = |entity: &EntityType| is_a(index, &entity.storage, &[CONFIG_STORAGE]);
        let found = self.entities.get(id);
        match storage {
            Storage::Any => !found.is_empty(),
            Storage::Config => found.iter().any(config),
            Storage::Content => found.iter().any(|entity| !config(entity)),
        }
    }

    /// Whether the service `id` is defined anywhere on disk: in a services file,
    /// test modules included, by the kernel, or possibly by PHP code that defines
    /// services.
    pub fn defines_service(&self, index: &Index, id: &str) -> bool {
        self.services.defines(id)
            || SYNTHETIC.contains(&id)
            || self.hooks.is_hook_class(index, id)
            || self.literals.services.contains(id)
            || self.literals.matches_service(id)
    }

    /// Whether the route `name` is defined anywhere on disk: in a routing file, test
    /// modules included, or possibly by PHP code that builds routes.
    pub fn defines_route(&self, name: &str) -> bool {
        self.routes.defines(name) || {
            self.literals.routes.contains(name) || self.literals.matches_route(name)
        }
    }

    /// Whether `manager` has a plugin `id` (or one of its bases, for a derivative id)
    /// anywhere on disk, test modules included, or possibly added by PHP code.
    pub fn defines_plugin(&self, manager: &str, id: &str) -> bool {
        self.plugins.defines(manager, id)
            || self.literals.plugins.contains(id)
            || plugins::derivative_bases(id).any(|base| self.literals.plugins.contains(base))
    }

    /// Every definition of the route `name`, sorted by path.
    pub fn routes(&self, name: &str) -> &[RouteDefinition] {
        self.routes.get(name)
    }

    /// Every plugin type, sorted by manager.
    pub fn plugin_types(&self) -> &[PluginType] {
        self.plugins.types()
    }

    /// Every plugin of `manager`, sorted by path and offset.
    pub fn plugins_of(&self, manager: &str) -> &[Plugin] {
        self.plugins.of(manager)
    }

    /// The plugins of `manager` an id names: that id, or for `base:derivative` with
    /// none, `base`.
    pub fn plugins(&self, manager: &str, id: &str) -> Vec<&Plugin> {
        self.plugins.named(manager, id)
    }

    /// Every implementation of `hook`, sorted by path and offset.
    pub fn hook_implementations(&self, hook: &str) -> &[HookImplementation] {
        self.hooks.implementations(hook)
    }

    /// Every `hook_{name}` documentation function.
    pub fn hook_docs(&self, name: &str) -> &[HookDoc] {
        self.hooks.docs(name)
    }

    /// Every declaration of the entity type `id`, outside test modules.
    pub fn entity_types(&self, id: &str) -> &[EntityType] {
        self.entities.get(id)
    }

    /// Every definition of `id` on disk.
    pub fn services(&self, id: &str) -> &[ServiceDefinition] {
        self.services.definitions(id)
    }

    /// The classes a service id can give, sorted: through aliases, parents, an id
    /// that names an indexed class, and every service decorating it. Empty when
    /// unknown.
    pub fn service_classes(&self, index: &Index, id: &str) -> Vec<String> {
        self.services.classes(index, id)
    }
}

impl Framework for Drupal {
    fn name(&self) -> &'static str {
        "drupal"
    }

    fn package(&self) -> &'static str {
        "drupal/core"
    }

    fn project(&self) -> &Project {
        &self.project
    }

    fn set_project(&mut self, project: Project) {
        self.project = project;
    }

    /// Contributed extensions, then core itself.
    fn dependency_directories(&self) -> &'static [&'static str] {
        &["contrib", "core"]
    }

    /// Extension files, include files, install files and the files of themes,
    /// profiles and theme engines.
    fn php_extensions(&self) -> &'static [&'static str] {
        &["module", "inc", "install", "theme", "profile", "engine"]
    }

    /// Reads files and declarations of the Drupal project only.
    fn rebuild(&mut self, index: &Index) {
        let filter = index.filter().scoped(&self.project);
        self.services = Services::read(index, &filter);
        self.entities = EntityTypes::read(index, &filter);
        self.hooks = Hooks::read(index, &filter);
        self.plugins = Plugins::read(index, &filter, self.hooks.extensions());
        self.routes = Routes::read(index, &filter);
        self.routes.read_attributes(index, &filter);
        self.literals = PhpLiterals::read(index, &filter);
    }

    /// Paths outside the Drupal project are dropped; a directory that holds the
    /// project is read as the project.
    fn update(&mut self, index: &Index, changed: &[PathBuf], removed: &[PathBuf]) {
        let filter = index.filter().scoped(&self.project);
        // A directory holding the project stands for the project itself.
        let ours = |paths: &[PathBuf]| -> Vec<PathBuf> {
            paths
                .iter()
                .filter_map(|path| {
                    let path = filter.under_root(path);
                    if self.project.contains(&path) {
                        Some(path)
                    } else if self.project.dir.starts_with(&path) {
                        Some(self.project.dir.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };
        let (changed, removed) = (ours(changed), ours(removed));
        self.services.update(index, &filter, &changed, &removed);
        self.entities = EntityTypes::read(index, &filter);
        self.hooks.update(index, &filter, &changed, &removed);
        self.plugins
            .update(index, &filter, self.hooks.extensions(), &changed, &removed);
        self.routes.update(index, &filter, &changed, &removed);
        self.routes.read_attributes(index, &filter);
        self.literals.update(&changed, &removed, index, &filter);
    }

    fn typed_methods(&self) -> &'static [&'static str] {
        &TYPED_METHODS
    }

    fn targeted_methods(&self) -> &'static [&'static str] {
        &[]
    }

    /// `\Drupal::service(id)` and a container's `get(id)` give the service's classes;
    /// `getStorage(id)` gives the entity type's storage, tagged with the entity type,
    /// and that storage's loading methods give the entity class.
    fn call_type(&self, index: &Index, call: &FrameworkCall<'_>) -> Option<TypeExpr> {
        if let Some(id) = service_call(index, call) {
            let classes = self.services.classes(index, id);
            return one_of(classes.into_iter().map(TypeExpr::Named).collect());
        }
        if let Some(id) = storage_call(index, call) {
            // A storage class the index lacks would hide the declared return type.
            return one_of(
                self.entities
                    .get(id)
                    .iter()
                    .filter(|entity| index.has_class(&entity.storage))
                    .map(|entity| TypeExpr::Tagged {
                        class: entity.storage.clone(),
                        tag: format!("{ENTITY_TAG}{id}"),
                    })
                    .collect(),
            );
        }
        let (ids, many) = entity_call(index, call)?;
        let entities: Vec<TypeExpr> = ids
            .iter()
            .flat_map(|id| self.entities.get(id))
            .map(|entity| TypeExpr::Named(entity.class.clone()))
            .collect();
        let entity = one_of(entities)?;
        Some(if many {
            TypeExpr::ArrayOf(Box::new(entity))
        } else {
            entity
        })
    }

    /// The id of a service call leads to every definition of the service; the id of
    /// `getStorage()` to the `id` of every declaration of the entity type; a hook
    /// name or alter type in an invocation to the hook's implementations and
    /// documentation.
    fn string_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        argument: usize,
        value: &str,
    ) -> Vec<(Location, String)> {
        if argument == 0 && service_call(index, call) == Some(value) {
            return self
                .services
                .definitions(value)
                .iter()
                .map(|definition| {
                    let location = Location {
                        path: definition.path.clone(),
                        start: definition.start,
                        end: definition.end,
                    };
                    (location, value.to_string())
                })
                .collect();
        }
        if argument == 0 && storage_call(index, call) == Some(value) {
            return self
                .entities
                .get(value)
                .iter()
                .filter_map(|entity| entity.id_location())
                .map(|location| (location, value.to_string()))
                .collect();
        }
        let routes = self.routes.call_targets(index, call, argument, value);
        if !routes.is_empty() {
            return routes;
        }
        let plugins = self.plugins.call_targets(index, call, argument, value);
        if !plugins.is_empty() {
            return plugins;
        }
        let entity_types = |id: &str, storage: Storage| self.has_entity_type(index, id, storage);
        self.hooks.invocation_targets(
            index,
            call.targets,
            call.strings,
            argument,
            value,
            &entity_types,
        )
    }

    fn attribute_targets(
        &self,
        _index: &Index,
        _found: &crate::frameworks::AttributeString,
        _offset: u32,
    ) -> Vec<(Location, String)> {
        Vec::new()
    }

    fn call_targets(
        &self,
        _index: &Index,
        _call: &FrameworkCall<'_>,
        _arguments: &[TypeExpr],
    ) -> Vec<(Location, String)> {
        Vec::new()
    }

    /// A render element type or a widget or formatter type in display options leads
    /// to its plugin class.
    fn keyed_string_targets(
        &self,
        index: &Index,
        key: &str,
        value: &str,
        call: Option<(&FrameworkCall<'_>, usize)>,
    ) -> Vec<(Location, String)> {
        self.plugins.keyed_targets(index, key, value, call)
    }

    /// A class, method, service or route referenced in a services, routing or links
    /// YAML file leads to its definition.
    fn file_targets(
        &self,
        index: &Index,
        path: &Path,
        source: &[u8],
        offset: u32,
    ) -> Vec<(Location, String)> {
        if !crate::filter::is_yaml(path) {
            return Vec::new();
        }
        navigation::targets(self, index, path, source, offset)
    }

    /// Service ids, route names and plugin ids in PHP defined nowhere on disk.
    fn diagnostics(&self, index: &Index, path: &Path, source: &[u8]) -> Vec<Diagnostic> {
        if crate::filter::is_framework_file(path) {
            return Vec::new();
        }
        broken::find(self, index, path, source)
    }

    /// Plugin discovery reads the annotations of a class in the subdirectory of a
    /// plugin type that has an annotation class.
    fn annotation_classes(
        &self,
        index: &Index,
        class: &str,
        name: &NameRef,
    ) -> Option<Vec<String>> {
        let annotations: Vec<&str> = self
            .plugins
            .types_of_class(class)
            .into_iter()
            .filter_map(|t| t.annotation.as_deref())
            .collect();
        if annotations.is_empty() {
            return None;
        }
        let short = name.fqn.rsplit('\\').next().unwrap_or(&name.fqn);
        Some(
            annotations
                .into_iter()
                .flat_map(|annotation| plugins::discovered_annotation_names(short, annotation))
                .filter(|candidate| index.has_class(candidate))
                .collect(),
        )
    }

    /// A plugin manager, or an interface only one plugin manager implements, has its
    /// plugins as implementations.
    fn class_implementations(&self, index: &Index, class: &str) -> Vec<(Location, String)> {
        self.plugins
            .manager_of(index, &[class.to_string()])
            .map(|manager| self.plugins.all_targets(&manager))
            .unwrap_or_default()
    }

    /// A hook implementation leads to the invocations of its hook and to the hook's
    /// documentation.
    fn declaration_targets(
        &self,
        index: &Index,
        declaration: &FrameworkDeclaration<'_>,
    ) -> Vec<(Location, String)> {
        let entity_types = |id: &str, storage: Storage| self.has_entity_type(index, id, storage);
        self.hooks
            .implementation_targets(index, &self.project, declaration, &entity_types)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

const CONFIG_STORAGE: &str = "Drupal\\Core\\Config\\Entity\\ConfigEntityStorage";

/// The tag a storage carries: this prefix, then the entity type id.
const ENTITY_TAG: &str = "drupal.entity:";

/// Storage methods that return one entity, and those that return several.
const LOAD_ONE: [&str; 4] = ["load", "loadUnchanged", "loadRevision", "create"];
/// Every method `call_type` reads, lowercased: service, storage and loading calls.
const TYPED_METHODS: [&str; 9] = [
    "service",
    "get",
    "getstorage",
    "load",
    "loadunchanged",
    "loadrevision",
    "create",
    "loadmultiple",
    "loadbyproperties",
];
const LOAD_MANY: [&str; 2] = ["loadMultiple", "loadByProperties"];

/// One type, or their union; nothing when there is none.
fn one_of(mut types: Vec<TypeExpr>) -> Option<TypeExpr> {
    match types.len() {
        0 => None,
        1 => types.pop(),
        _ => Some(TypeExpr::Union(types)),
    }
}

/// The entity type id `call` asks for, when it is `getStorage(id)` on an entity type
/// manager with a literal id.
fn storage_call<'a>(index: &Index, call: &'a FrameworkCall<'_>) -> Option<&'a str> {
    let id = call.strings.first()?.as_deref()?;
    let asks_for_storage = call.targets.iter().any(|target| match target {
        CallTarget::Method { class, name } => {
            name.eq_ignore_ascii_case("getStorage")
                && is_a(
                    index,
                    class,
                    &["Drupal\\Core\\Entity\\EntityTypeManagerInterface"],
                )
        }
        CallTarget::Function(_) => false,
    });
    asks_for_storage.then_some(id)
}

/// The entity type ids of a loading call on a tagged storage, and whether it loads
/// several entities.
fn entity_call(index: &Index, call: &FrameworkCall<'_>) -> Option<(Vec<String>, bool)> {
    let mut ids = Vec::new();
    collect_tags(call.receiver?, &mut ids);
    if ids.is_empty() {
        return None;
    }
    let mut many = None;
    for target in call.targets {
        let CallTarget::Method { class, name } = target else {
            continue;
        };
        if !is_a(
            index,
            class,
            &["Drupal\\Core\\Entity\\EntityStorageInterface"],
        ) {
            continue;
        }
        if LOAD_ONE.iter().any(|m| m.eq_ignore_ascii_case(name)) {
            many = Some(false);
        } else if LOAD_MANY.iter().any(|m| m.eq_ignore_ascii_case(name)) {
            many = Some(true);
        }
    }
    Some((ids, many?))
}

/// The entity type ids tagged anywhere in `ty`.
fn collect_tags(ty: &TypeExpr, ids: &mut Vec<String>) {
    match ty {
        TypeExpr::Tagged { tag, .. } => {
            if let Some(id) = tag.strip_prefix(ENTITY_TAG)
                && !ids.iter().any(|known| known == id)
            {
                ids.push(id.to_string());
            }
        }
        TypeExpr::Union(members) => members.iter().for_each(|m| collect_tags(m, ids)),
        _ => {}
    }
}

/// Container interfaces whose `get(id)` returns a service.
const CONTAINERS: [&str; 2] = [
    "Symfony\\Component\\DependencyInjection\\ContainerInterface",
    "Psr\\Container\\ContainerInterface",
];

/// The service id `call` asks for, when it is `\Drupal::service(id)` or a
/// container's `get(id)` with a literal id.
fn service_call<'a>(index: &Index, call: &'a FrameworkCall<'_>) -> Option<&'a str> {
    let id = call.strings.first()?.as_deref()?;
    let asks_for_service = call.targets.iter().any(|target| match target {
        CallTarget::Method { class, name } => {
            (class.eq_ignore_ascii_case("Drupal") && name.eq_ignore_ascii_case("service"))
                || (name.eq_ignore_ascii_case("get") && is_a(index, class, &CONTAINERS))
        }
        CallTarget::Function(_) => false,
    });
    asks_for_service.then_some(id)
}
