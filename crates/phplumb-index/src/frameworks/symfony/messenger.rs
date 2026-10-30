//! Symfony Messenger: the message handlers `#[AsMessageHandler]` declares or the
//! configuration tags, and the messages each one handles.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::config::Tagged;
use crate::filter::PathFilter;
use crate::frameworks::ancestry::{is_a, methods_of};
use crate::index::{Index, Location};
use crate::infer::class_names;
use crate::metadata::{MetaValue, Metadata};
use crate::summary::{Declaration, Member, MemberKind, SymbolKind};

/// The attribute that marks a message handler.
const HANDLER_ATTRIBUTE: &str = "Symfony\\Component\\Messenger\\Attribute\\AsMessageHandler";

/// A handler method, its name and the message classes it handles.
struct Handler {
    location: Location,
    name: String,
    handles: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Handlers {
    list: Vec<Handler>,
}

impl Handlers {
    /// Every handler of the concrete project classes: a class attribute names its
    /// method (`__invoke` by default), a method attribute marks its method, also on
    /// a method the class takes from a parent or a trait; then the handlers of the
    /// services `tagged` `messenger.message_handler`.
    pub(crate) fn read(index: &Index, filter: &PathFilter, tagged: &[Tagged]) -> Handlers {
        let mut handlers = Handlers::default();
        let declaring: HashSet<String> = index
            .declarations()
            .filter(|(_, declaration)| declares_handler_methods(declaration))
            .map(|(_, declaration)| declaration.fqn.to_ascii_lowercase())
            .collect();
        let mut inherits: HashMap<String, bool> = HashMap::new();
        for (path, declaration) in index.declarations() {
            if declaration.kind == SymbolKind::Class
                && (declares_handlers(declaration)
                    || (!declaring.is_empty()
                        && inherits_handlers(index, declaration, &declaring, &mut inherits)))
                && filter.in_scope(path)
                && index.is_project_code(path)
            {
                handlers.read_class(index, path, declaration);
            }
        }
        for service in tagged {
            if service.instanceof {
                for (path, declaration) in index.declarations() {
                    if declaration.kind == SymbolKind::Class
                        && filter.in_scope(path)
                        && index.is_project_code(path)
                        && is_a(index, &declaration.fqn, &[&service.class])
                    {
                        handlers.read_tagged(index, path, declaration, &service.attributes);
                    }
                }
            } else if let Some(found) = index.classes_named(&service.class).first() {
                handlers.read_tagged(index, found.path, found.declaration, &service.attributes);
            }
        }
        handlers
    }

    /// The handler a `messenger.message_handler` tag with `attributes` makes of
    /// `class`: its `method` (`__invoke` by default), handling the tag's `handles`
    /// or the type of the method's first parameter.
    fn read_tagged(
        &mut self,
        index: &Index,
        path: &Path,
        class: &Declaration,
        attributes: &[(String, String)],
    ) {
        if class.class.as_ref().is_none_or(|shape| shape.is_abstract) {
            return;
        }
        let attribute = |name: &str| {
            attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let name = attribute("method").unwrap_or("__invoke");
        let methods = methods_of(index, path, class);
        if let Some((file, _, method)) = methods
            .iter()
            .find(|(_, _, m)| m.name.eq_ignore_ascii_case(name))
        {
            self.add(file, method, attribute("handles").map(str::to_string));
        }
    }

    fn read_class(&mut self, index: &Index, path: &Path, class: &Declaration) {
        let Some(shape) = &class.class else {
            return;
        };
        if shape.is_abstract {
            return;
        }
        let methods = methods_of(index, path, class);
        for attribute in shape.metadata.iter().filter(|m| is_handler(m)) {
            let name = match argument(attribute, "method", 3) {
                Some(MetaValue::String(method)) => method.as_str(),
                Some(_) => continue,
                None => "__invoke",
            };
            if let Some((file, _, method)) = methods
                .iter()
                .find(|(_, _, m)| m.name.eq_ignore_ascii_case(name))
            {
                self.add_attributed(file, method, attribute);
            }
        }
        for (file, _, method) in &methods {
            for attribute in method.metadata.iter().filter(|m| is_handler(m)) {
                self.add_attributed(file, method, attribute);
            }
        }
    }

    /// The handler an `#[AsMessageHandler]` `attribute` makes of `method`.
    fn add_attributed(&mut self, path: &Path, method: &Member, attribute: &Metadata) {
        let handles = match argument(attribute, "handles", 2) {
            Some(MetaValue::Class(class) | MetaValue::String(class)) => Some(class.clone()),
            Some(_) => return,
            None => None,
        };
        self.add(path, method, handles);
    }

    /// The handler `method`, handling the class `handles` names or the classes of
    /// its first parameter's declared type, as `MessengerPass` reads it. Symfony
    /// registers public methods only.
    fn add(&mut self, path: &Path, method: &Member, handles: Option<String>) {
        if !is_public(method) {
            return;
        }
        let handles = match handles {
            Some(class) => vec![class.trim_start_matches('\\').to_string()],
            None => method
                .hints
                .first()
                .and_then(Option::as_ref)
                .map(class_names)
                .unwrap_or_default(),
        };
        if handles.is_empty() {
            return;
        }
        self.list.push(Handler {
            location: Location {
                path: path.to_path_buf(),
                start: method.name_start,
                end: method.name_end,
            },
            name: method.name.clone(),
            handles,
        });
    }

    /// The handlers whose handled class `message` is, extends or implements, or,
    /// for an interface or abstract class, one of its subtypes, each with its method
    /// name.
    pub(crate) fn of(&self, index: &Index, message: &str) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = Vec::new();
        for handler in &self.list {
            let handles = handler
                .handles
                .iter()
                .any(|class| may_carry(index, message, class));
            if handles && !found.iter().any(|(known, _)| *known == handler.location) {
                found.push((handler.location.clone(), handler.name.clone()));
            }
        }
        found
    }

    /// The classes the handler method whose name starts at `start` in `path`
    /// handles; none when it is no handler.
    pub(crate) fn handled_by(&self, path: &Path, start: u32) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for handler in &self.list {
            if handler.location.path == path && handler.location.start == start {
                for class in &handler.handles {
                    if !found.contains(class) {
                        found.push(class.clone());
                    }
                }
            }
        }
        found
    }
}

/// Whether a message typed `message` may be an instance of `class`: `message` is,
/// extends or implements it, or, being an interface or abstract class, is one of
/// its supertypes. `class` may be a wildcard, `*` for every message and `Ns\*` for
/// the classes of a namespace and those below it, as `HandlersLocator` lists them.
pub(crate) fn may_carry(index: &Index, message: &str, class: &str) -> bool {
    if class == "*" {
        return true;
    }
    if let Some(namespace) = class.replace("\\\\", "\\").strip_suffix('*') {
        return message.len() > namespace.len()
            && message
                .get(..namespace.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(namespace));
    }
    is_a(index, message, &[class]) || (is_open(index, message) && is_a(index, class, &[message]))
}

/// Whether `class` is an interface or an abstract class, which no message is an
/// instance of itself.
fn is_open(index: &Index, class: &str) -> bool {
    index.classes_named(class).first().is_some_and(|found| {
        found.declaration.kind == SymbolKind::Interface
            || found
                .declaration
                .class
                .as_ref()
                .is_some_and(|shape| shape.is_abstract)
    })
}

/// The method a class-level `#[AsMessageHandler]` names, when `attribute` is one.
pub(crate) fn handler_method<'a>(
    class: &'a Declaration,
    attribute: &Metadata,
) -> Option<&'a Member> {
    if !is_handler(attribute) {
        return None;
    }
    let name = match argument(attribute, "method", 3) {
        Some(MetaValue::String(method)) => method.as_str(),
        Some(_) => return None,
        None => "__invoke",
    };
    class
        .class
        .as_ref()?
        .members
        .iter()
        .find(|m| m.kind == MemberKind::Method && m.name.eq_ignore_ascii_case(name))
}

/// Whether `method` is public: declared without `private` or `protected`.
fn is_public(method: &Member) -> bool {
    let modifiers = method
        .signature
        .split_once("function")
        .map_or("", |(before, _)| before)
        .to_ascii_lowercase();
    !method.is_private && !modifiers.contains("protected") && !modifiers.contains("private")
}

fn is_handler(metadata: &Metadata) -> bool {
    !metadata.annotation && metadata.name.eq_ignore_ascii_case(HANDLER_ATTRIBUTE)
}

/// Whether one of the methods `class` declares carries `#[AsMessageHandler]`.
fn declares_handler_methods(class: &Declaration) -> bool {
    class.class.as_ref().is_some_and(|shape| {
        shape
            .members
            .iter()
            .any(|m| m.kind == MemberKind::Method && m.metadata.iter().any(is_handler))
    })
}

/// Whether a parent or a trait of `class`, or one of theirs, is in `declaring`,
/// memoized by lowercased name.
fn inherits_handlers(
    index: &Index,
    class: &Declaration,
    declaring: &HashSet<String>,
    memo: &mut HashMap<String, bool>,
) -> bool {
    let key = class.fqn.to_ascii_lowercase();
    if let Some(&known) = memo.get(&key) {
        return known;
    }
    // A cycle in the hierarchy reads as no handlers.
    memo.insert(key.clone(), false);
    let Some(shape) = &class.class else {
        return false;
    };
    let found = shape.parent.iter().chain(&shape.traits).any(|name| {
        declaring.contains(&name.to_ascii_lowercase())
            || index.classes_named(name).first().is_some_and(|ancestor| {
                inherits_handlers(index, ancestor.declaration, declaring, memo)
            })
    });
    memo.insert(key, found);
    found
}

/// Whether the class or one of its methods carries `#[AsMessageHandler]`.
fn declares_handlers(class: &Declaration) -> bool {
    class.class.as_ref().is_some_and(|shape| {
        shape.metadata.iter().any(is_handler)
            || shape
                .members
                .iter()
                .any(|m| m.metadata.iter().any(is_handler))
    })
}

/// The argument `name`, given by name or at `position`.
fn argument<'a>(attribute: &'a Metadata, name: &str, position: usize) -> Option<&'a MetaValue> {
    attribute.argument(name).or_else(|| {
        attribute
            .arguments
            .get(position)
            .filter(|(key, _)| key.is_none())
            .map(|(_, value)| value)
    })
}
