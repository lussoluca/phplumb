//! Drupal plugins: plugin types read from the constructors of plugin managers, and
//! the plugin classes of each type, found by attribute or annotation in the type's
//! subdirectory of every module namespace and of every `Drupal\Core\*` and
//! `Drupal\Component\*` namespace.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::extensions::Extensions;
use crate::filter::PathFilter;
use crate::frameworks::ancestry::is_a;
use crate::frameworks::{CallTarget, FrameworkCall, under_any};
use crate::index::{Index, Location};
use crate::infer::{self, class_names};
use crate::metadata::{MetaValue, Metadata};
use crate::summary::{MemberKind, SymbolKind, summarize};

pub struct PluginType {
    /// The plugin manager class.
    pub manager: String,
    pub subdir: String,
    /// The attribute class plugins declare themselves with; none when the manager
    /// reads annotations only.
    pub attribute: Option<String>,
    /// The annotation class plugins declare themselves with; none when the manager
    /// reads attributes only.
    pub annotation: Option<String>,
    /// The lookup methods (`createInstance`, `getDefinition`, `getDefinitions`,
    /// `getDiscovery`) the manager gets from somewhere else than the default manager
    /// does, as written in `LOOKUPS`.
    pub overrides: Vec<&'static str>,
    /// The manager sets its discovery itself.
    pub sets_discovery: bool,
}

impl PluginType {
    /// Whether the manager may find a plugin id its own way when it is looked up
    /// through `method`, or by any means when no method is given: it sets its own
    /// discovery, overrides how definitions are found, or overrides `method`.
    pub(crate) fn finds_its_own_way(&self, method: Option<&str>) -> bool {
        self.sets_discovery
            || self.overrides.iter().any(|o| {
                ["getDefinition", "getDefinitions", "getDiscovery"].contains(o)
                    || method.is_some_and(|m| m.eq_ignore_ascii_case(o))
            })
    }
}

/// Methods a manager declares when it looks plugins up its own way.
const LOOKUPS: [&str; 4] = [
    "createInstance",
    "getDefinition",
    "getDefinitions",
    "getDiscovery",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub id: String,
    pub manager: String,
    pub class: String,
    pub path: PathBuf,
    /// The class name.
    pub start: u32,
    pub end: u32,
}

const DEFAULT_MANAGER: &str = "Drupal\\Core\\Plugin\\DefaultPluginManager";
const FIELD_TYPES: &str = "Drupal\\Core\\Field\\FieldTypePluginManager";
const CONSTRAINTS: &str = "Drupal\\Core\\Validation\\ConstraintManager";
pub(crate) const ELEMENTS: &str = "Drupal\\Core\\Render\\ElementInfoManager";
const WIDGETS: &str = "Drupal\\Core\\Field\\WidgetPluginManager";
const FORMATTERS: &str = "Drupal\\Core\\Field\\FormatterPluginManager";
const BASE_FIELD: &str = "Drupal\\Core\\Field\\BaseFieldDefinition";
/// Classes whose `addConstraint` takes a constraint plugin id.
const CONSTRAINED: [&str; 3] = [
    "Drupal\\Core\\Entity\\EntityTypeInterface",
    "Drupal\\Core\\TypedData\\DataDefinitionInterface",
    "Drupal\\Core\\Field\\FieldConfigInterface",
];
/// Plugin manager methods whose first argument is a plugin id.
const BY_ID: [&str; 3] = ["createInstance", "getDefinition", "hasDefinition"];
const ATTRIBUTE_INTERFACE: &str = "Drupal\\Component\\Plugin\\Attribute\\AttributeInterface";
const PLUGIN_MANAGER: &str = "Drupal\\Component\\Plugin\\PluginManagerInterface";
/// Where unimported annotations resolve besides the annotation class's namespace.
/// The namespace core plugin discovery reads annotations in for every plugin type.
const CORE_ANNOTATIONS: &str = "Drupal\\Core\\Annotation";
const DEFAULT_ANNOTATION: &str = "Drupal\\Component\\Annotation\\Plugin";

/// Plugin types, and the plugins of each by lowercased manager.
#[derive(Default)]
pub(crate) struct Plugins {
    types: Vec<PluginType>,
    /// The files declaring plugin managers.
    manager_files: HashSet<PathBuf>,
    by_manager: HashMap<String, Vec<Plugin>>,
    /// Plugin ids of test modules by lowercased manager, which navigation ignores.
    test_ids: HashMap<String, HashSet<String>>,
    /// Lowercased managers with a plugin whose id the registry cannot read, such as
    /// a class constant.
    uncertain: HashSet<String>,
}

impl Plugins {
    pub(crate) fn read(index: &Index, filter: &PathFilter, extensions: &Extensions) -> Plugins {
        let mut plugins = Plugins::default();
        plugins.read_types(index, filter);
        plugins.collect(index, filter, extensions);
        plugins
    }

    /// Reads the plugin types again when a manager's file or a file declaring a
    /// manager changed, then collects the plugins again from the index.
    pub(crate) fn update(
        &mut self,
        index: &Index,
        filter: &PathFilter,
        extensions: &Extensions,
        changed: &[PathBuf],
        removed: &[PathBuf],
    ) {
        let gone: HashSet<PathBuf> = removed.iter().chain(changed).cloned().collect();
        let changed: HashSet<PathBuf> = changed.iter().cloned().collect();
        let touches_managers = self.manager_files.iter().any(|file| under_any(file, &gone))
            || index.declarations().any(|(path, declaration)| {
                declaration.kind == SymbolKind::Class
                    && under_any(path, &changed)
                    && filter.in_scope(path)
                    && is_a(index, &declaration.fqn, &[DEFAULT_MANAGER])
            });
        if touches_managers {
            self.read_types(index, filter);
        }
        self.collect(index, filter, extensions);
    }

    pub(crate) fn types(&self) -> &[PluginType] {
        &self.types
    }

    /// The plugin types whose subdirectory holds `class`, by the namespace rules of
    /// plugin discovery.
    pub(crate) fn types_of_class(&self, class: &str) -> Vec<&PluginType> {
        let lower = class.to_ascii_lowercase();
        let Some((extension, rest)) = lower
            .strip_prefix("drupal\\")
            .and_then(|rest| rest.split_once('\\'))
        else {
            return Vec::new();
        };
        let rest = if extension == "core" || extension == "component" {
            rest.split_once('\\').map_or("", |(_, below)| below)
        } else {
            rest
        };
        self.types
            .iter()
            .filter(|t| {
                let namespace = t.subdir.trim_matches('/').replace('/', "\\");
                rest.strip_prefix(&namespace.to_ascii_lowercase())
                    .is_some_and(|below| below.starts_with('\\'))
            })
            .collect()
    }

    pub(crate) fn of(&self, manager: &str) -> &[Plugin] {
        self.by_manager
            .get(&manager.to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }

    /// The plugins of `manager` named `id`; for a derivative id with none, those
    /// named after its longest base that has some.
    pub(crate) fn named(&self, manager: &str, id: &str) -> Vec<&Plugin> {
        let exact: Vec<&Plugin> = self.of(manager).iter().filter(|p| p.id == id).collect();
        if !exact.is_empty() {
            return exact;
        }
        derivative_bases(id)
            .map(|base| -> Vec<&Plugin> {
                self.of(manager).iter().filter(|p| p.id == base).collect()
            })
            .find(|found| !found.is_empty())
            .unwrap_or_default()
    }

    /// The plugin classes a plugin id at position `argument` of `call` names, each
    /// with its class name as the expected text.
    pub(crate) fn call_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        argument: usize,
        value: &str,
    ) -> Vec<(Location, String)> {
        self.call_manager(index, call, argument)
            .map_or_else(Vec::new, |manager| self.targets(&manager, value))
    }

    /// The plugin manager whose plugin id position `argument` of `call` takes, by the
    /// call contexts of the id context table.
    pub(crate) fn call_manager(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        argument: usize,
    ) -> Option<String> {
        if argument != 0 {
            return None;
        }
        let named = |wanted: &str| -> Vec<&str> {
            call.targets
                .iter()
                .filter_map(|target| match target {
                    CallTarget::Method { class, name } if name.eq_ignore_ascii_case(wanted) => {
                        Some(class.as_str())
                    }
                    _ => None,
                })
                .collect()
        };
        if BY_ID.iter().any(|m| !named(m).is_empty()) {
            call.receiver
                .and_then(|receiver| self.manager_of(index, &class_names(receiver)))
        } else if named("create")
            .iter()
            .any(|class| class.eq_ignore_ascii_case(BASE_FIELD))
        {
            Some(FIELD_TYPES.to_string())
        } else if named("addConstraint")
            .iter()
            .any(|class| is_a(index, class, &CONSTRAINED))
        {
            Some(CONSTRAINTS.to_string())
        } else {
            None
        }
    }

    /// The plugin classes a string literal names as the value of the array key `key`.
    pub(crate) fn keyed_targets(
        &self,
        index: &Index,
        key: &str,
        value: &str,
        call: Option<(&FrameworkCall<'_>, usize)>,
    ) -> Vec<(Location, String)> {
        keyed_manager(index, key, call)
            .map_or_else(Vec::new, |manager| self.targets(manager, value))
    }

    /// The plugin manager that a receiver of these classes is: a manager class, or an
    /// interface exactly one manager implements; nothing when that is not one
    /// manager.
    pub(crate) fn manager_of(&self, index: &Index, classes: &[String]) -> Option<String> {
        let mut found: Vec<&str> = Vec::new();
        for class in classes {
            let managers: Vec<&str> = match self
                .types
                .iter()
                .find(|t| t.manager.eq_ignore_ascii_case(class))
            {
                Some(own) => vec![own.manager.as_str()],
                None if is_a(index, class, &[PLUGIN_MANAGER]) => self
                    .types
                    .iter()
                    .filter(|t| is_a(index, &t.manager, &[class.as_str()]))
                    .map(|t| t.manager.as_str())
                    .collect(),
                None => return None,
            };
            if managers.len() != 1 {
                return None;
            }
            if !found.contains(&managers[0]) {
                found.push(managers[0]);
            }
        }
        (found.len() == 1).then(|| found[0].to_string())
    }

    /// The plugin classes of `manager` an id names, each with its class name as the
    /// expected text.
    pub(crate) fn targets(&self, manager: &str, id: &str) -> Vec<(Location, String)> {
        to_targets(self.named(manager, id))
    }

    /// Every plugin class of `manager`, each with its class name as the expected text.
    pub(crate) fn all_targets(&self, manager: &str) -> Vec<(Location, String)> {
        to_targets(self.of(manager))
    }

    /// Every subclass of the default plugin manager whose own constructor calls the
    /// default constructor with a literal subdirectory.
    fn read_types(&mut self, index: &Index, filter: &PathFilter) {
        let mut types = Vec::new();
        let mut files = HashSet::new();
        for manager in index.all_subtypes(DEFAULT_MANAGER) {
            if !filter.in_scope(manager.path) || manager.declaration.kind != SymbolKind::Class {
                continue;
            }
            let fqn = &manager.declaration.fqn;
            let Ok(source) = std::fs::read(manager.path) else {
                continue;
            };
            let summary = summarize(&source);
            let caller = format!("{fqn}::__construct");
            let calls =
                infer::calls_named(index, manager.path, &summary, &source, &["__construct"]);
            let Some(call) = calls.iter().find(|call| {
                call.caller
                    .as_deref()
                    .is_some_and(|c| c.eq_ignore_ascii_case(&caller))
                    && call.targets.iter().any(|target| {
                        matches!(target, CallTarget::Method { class, .. } if class.eq_ignore_ascii_case(DEFAULT_MANAGER))
                    })
            }) else {
                continue;
            };
            let argument = |position: usize| call.strings.get(position).cloned().flatten();
            let Some(subdir) = argument(0) else {
                continue;
            };
            files.insert(manager.path.to_path_buf());
            let class = |position: usize| {
                argument(position).map(|name| name.trim_start_matches('\\').to_string())
            };
            // As the default constructor does: argument 4 names an attribute when it
            // is one, else it is the annotation of a manager reading annotations only.
            let (attribute, annotation) = match class(4) {
                Some(attribute) if is_attribute(index, &attribute) => (Some(attribute), class(5)),
                fourth => (
                    None,
                    Some(fourth.unwrap_or_else(|| DEFAULT_ANNOTATION.to_string())),
                ),
            };
            let text = String::from_utf8_lossy(&source);
            // A lookup method counts when the manager gets it from somewhere else than
            // the default manager does.
            let declarer = |class: &str, method: &str| {
                index
                    .find_member(class, MemberKind::Method, method)
                    .first()
                    .map(|(declaring, _)| declaring.declaration.fqn.to_ascii_lowercase())
            };
            let overrides = LOOKUPS
                .iter()
                .copied()
                .filter(|method| declarer(fqn, method) != declarer(DEFAULT_MANAGER, method))
                .collect();
            types.push(PluginType {
                manager: fqn.clone(),
                subdir,
                attribute,
                annotation,
                overrides,
                sets_discovery: text.contains("->discovery ="),
            });
        }
        types.sort_by(|a, b| a.manager.cmp(&b.manager));
        types.dedup_by(|a, b| a.manager.eq_ignore_ascii_case(&b.manager));
        self.types = types;
        self.manager_files = files;
    }

    /// The plugins of every type: classes in the type's subdirectory of a module,
    /// core or component namespace, outside test modules, with the type's attribute
    /// or, failing that, its annotation.
    fn collect(&mut self, index: &Index, filter: &PathFilter, extensions: &Extensions) {
        let mut by_subdir: HashMap<String, Vec<&PluginType>> = HashMap::new();
        for plugin_type in &self.types {
            let namespace = plugin_type.subdir.trim_matches('/').replace('/', "\\");
            by_subdir
                .entry(namespace.to_ascii_lowercase())
                .or_default()
                .push(plugin_type);
        }
        let roots: HashSet<String> = extensions
            .modules
            .iter()
            .map(|m| m.to_ascii_lowercase())
            .chain(["core".to_string(), "component".to_string()])
            .collect();
        let root = filter.root().to_path_buf();
        let mut is_a_cache: HashMap<(String, String), bool> = HashMap::new();
        let mut found: HashMap<String, Vec<Plugin>> = HashMap::new();
        let mut test_ids: HashMap<String, HashSet<String>> = HashMap::new();
        let mut uncertain: HashSet<String> = HashSet::new();
        for (path, declaration) in index.declarations() {
            if !filter.in_scope(path) || declaration.kind != SymbolKind::Class {
                continue;
            }
            let tests = in_tests(&root, path);
            let Some(shape) = &declaration.class else {
                continue;
            };
            let lower = declaration.fqn.to_ascii_lowercase();
            let Some(rest) = lower.strip_prefix("drupal\\") else {
                continue;
            };
            let Some((extension, rest)) = rest.split_once('\\') else {
                continue;
            };
            // Test modules are not in the extension list; any namespace below
            // `Drupal\` may be one.
            if !roots.contains(extension) && !tests {
                continue;
            }
            // Core and component plugins live in a namespace one level below,
            // `Drupal\Core\{component}`, as the kernel registers them.
            let rest = if extension == "core" || extension == "component" {
                match rest.split_once('\\') {
                    Some((_, below)) => below,
                    None => continue,
                }
            } else {
                rest
            };
            let mut prefixes = Vec::new();
            let mut end = 0;
            while let Some(next) = rest[end..].find('\\') {
                end += next;
                prefixes.push(&rest[..end]);
                end += 1;
            }
            for prefix in prefixes {
                for plugin_type in by_subdir.get(prefix).into_iter().flatten() {
                    let mut matches = |name: &str, target: &str| {
                        if name.eq_ignore_ascii_case(target) {
                            return true;
                        }
                        *is_a_cache
                            .entry((name.to_ascii_lowercase(), target.to_ascii_lowercase()))
                            .or_insert_with(|| is_a(index, name, &[target]))
                    };
                    let attribute = plugin_type.attribute.as_deref().and_then(|target| {
                        shape
                            .metadata
                            .iter()
                            .find(|m| !m.annotation && matches(&m.name, target))
                    });
                    let declared = attribute.or_else(|| {
                        let target = plugin_type.annotation.as_deref()?;
                        shape.metadata.iter().find(|m| {
                            m.annotation
                                && annotation_names(m, target)
                                    .iter()
                                    .any(|name| matches(name, target))
                        })
                    });
                    let Some(id) = declared.and_then(plugin_id) else {
                        if declared.is_some() {
                            uncertain.insert(plugin_type.manager.to_ascii_lowercase());
                        }
                        continue;
                    };
                    if tests {
                        test_ids
                            .entry(plugin_type.manager.to_ascii_lowercase())
                            .or_default()
                            .insert(id.to_string());
                        continue;
                    }
                    found
                        .entry(plugin_type.manager.to_ascii_lowercase())
                        .or_default()
                        .push(Plugin {
                            id: id.to_string(),
                            manager: plugin_type.manager.clone(),
                            class: declaration.fqn.clone(),
                            path: path.to_path_buf(),
                            start: declaration.name_start,
                            end: declaration.name_end,
                        });
                }
            }
        }
        for plugins in found.values_mut() {
            plugins.sort_by(|a, b| (&a.path, a.start).cmp(&(&b.path, b.start)));
        }
        self.by_manager = found;
        self.test_ids = test_ids;
        self.uncertain = uncertain;
    }

    /// Whether `manager` has a plugin `id`, or one of its bases for a derivative id,
    /// test modules included; always, when the manager has a plugin whose id the
    /// registry cannot read.
    pub(crate) fn defines(&self, manager: &str, id: &str) -> bool {
        let manager = manager.to_ascii_lowercase();
        if self.uncertain.contains(&manager) || !self.named(&manager, id).is_empty() {
            return true;
        }
        let Some(ids) = self.test_ids.get(&manager) else {
            return false;
        };
        ids.contains(id) || derivative_bases(id).any(|base| ids.contains(base))
    }
}

/// The bases of a derivative id, longest first: `a:b:c` gives `a:b`, then `a`.
pub(crate) fn derivative_bases(id: &str) -> impl Iterator<Item = &str> {
    id.rmatch_indices(':').map(move |(at, _)| &id[..at])
}

/// Whether a class named as a manager's argument 4 is an attribute: it is or
/// extends the attribute interface, or, when it is not indexed, it is not in an
/// `Annotation` namespace.
fn is_attribute(index: &Index, class: &str) -> bool {
    if index.has_class(class) {
        return is_a(index, class, &[ATTRIBUTE_INTERFACE]);
    }
    !class.to_ascii_lowercase().contains("\\annotation\\")
}

/// The classes an annotation on a plugin of a type whose annotation class is
/// `target` can be: the class an explicit name names, or the classes plugin
/// discovery reads an unimported name as.
fn annotation_names(annotation: &Metadata, target: &str) -> Vec<String> {
    if annotation.explicit {
        return vec![annotation.name.clone()];
    }
    let short = annotation
        .name
        .rsplit('\\')
        .next()
        .unwrap_or(&annotation.name);
    discovered_annotation_names(short, target).to_vec()
}

/// The classes an unqualified annotation name the file does not import is on a
/// plugin of a type whose annotation class is `annotation`, as plugin discovery
/// reads it: the name in that class's namespace and in `Drupal\Core\Annotation`,
/// never in the plugin's own namespace.
pub(crate) fn discovered_annotation_names(short: &str, annotation: &str) -> [String; 2] {
    let own = annotation
        .rsplit_once('\\')
        .map_or("", |(namespace, _)| namespace);
    [
        format!("{own}\\{short}"),
        format!("{CORE_ANNOTATIONS}\\{short}"),
    ]
}

/// The plugin manager whose plugin id the value of the array key `key` is: render
/// elements for `#type`; widgets or formatters for `type` in the options of
/// `setDisplayOptions('form' or 'view', ...)` on a base field definition.
pub(crate) fn keyed_manager(
    index: &Index,
    key: &str,
    call: Option<(&FrameworkCall<'_>, usize)>,
) -> Option<&'static str> {
    if key == "#type" {
        return Some(ELEMENTS);
    }
    let (call, 1) = call.filter(|_| key == "type")? else {
        return None;
    };
    let displays = call.targets.iter().any(|target| {
        matches!(target, CallTarget::Method { class, name }
            if name.eq_ignore_ascii_case("setDisplayOptions") && is_a(index, class, &[BASE_FIELD]))
    });
    if !displays {
        return None;
    }
    match call.strings.first().cloned().flatten().as_deref() {
        Some("form") => Some(WIDGETS),
        Some("view") => Some(FORMATTERS),
        _ => None,
    }
}

/// Plugin classes as targets, each with its class name as the expected text.
fn to_targets<'a>(plugins: impl IntoIterator<Item = &'a Plugin>) -> Vec<(Location, String)> {
    plugins
        .into_iter()
        .map(|plugin| {
            let location = Location {
                path: plugin.path.clone(),
                start: plugin.start,
                end: plugin.end,
            };
            let name = plugin.class.rsplit('\\').next().unwrap_or(&plugin.class);
            (location, name.to_string())
        })
        .collect()
}

/// The `id` argument of a plugin attribute or annotation: named, or the first
/// positional one.
fn plugin_id(metadata: &Metadata) -> Option<&str> {
    metadata
        .argument("id")
        .or_else(|| {
            metadata
                .arguments
                .first()
                .filter(|(key, _)| key.is_none())
                .map(|(_, value)| value)
        })
        .and_then(|value| match value {
            MetaValue::String(id) => Some(id.as_str()),
            _ => None,
        })
}

fn in_tests(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .is_ok_and(|relative| relative.components().any(|c| c.as_os_str() == "tests"))
}
