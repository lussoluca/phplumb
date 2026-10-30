//! Doctrine mappings: the `*.orm.xml` files of the project and
//! `#[ORM\Entity(repositoryClass:)]` attributes, read for navigation and for the
//! repository `getRepository()` returns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::config::top_blocks;
use super::files::ConfigFiles;
use crate::filter::{PathFilter, is_doctrine_xml};
use crate::frameworks::yaml::{Step, Yaml, scalar_at};
use crate::index::Index;
use crate::metadata::MetaValue;
use crate::summary::SymbolKind;

/// The attribute that maps an entity class.
const ENTITY_ATTRIBUTE: &str = "Doctrine\\ORM\\Mapping\\Entity";

/// The attribute that maps a mapped superclass, whose repository its entities take.
const MAPPED_SUPERCLASS_ATTRIBUTE: &str = "Doctrine\\ORM\\Mapping\\MappedSuperclass";

/// An XML element of a mapping file: its name, its attributes with each value and
/// the value's range inside the quotes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Element {
    pub(crate) name: String,
    pub(crate) attributes: Vec<(String, String, u32, u32)>,
    /// Where the element's start tag ends, after its `>`.
    pub(crate) end: u32,
}

impl Element {
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, ..)| key == name)
            .map(|(_, value, ..)| value.as_str())
    }
}

/// The elements of `text` in order; comments, declarations and text are skipped.
pub(crate) fn elements(text: &str) -> Vec<Element> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        let skip_to = |end: &str| {
            text[i..]
                .find(end)
                .map_or(bytes.len(), |p| i + p + end.len())
        };
        if rest.starts_with("<!--") {
            i = skip_to("-->");
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = skip_to("]]>");
            continue;
        }
        if rest.starts_with("<?") {
            i = skip_to("?>");
            continue;
        }
        if rest.starts_with("<!") {
            i = skip_to(">");
            continue;
        }
        if rest.starts_with("</") {
            i = skip_to(">");
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && !bytes[j].is_ascii_whitespace() && !matches!(bytes[j], b'/' | b'>')
        {
            j += 1;
        }
        let name = text[i + 1..j].to_string();
        let mut attributes = Vec::new();
        while j < bytes.len() {
            match bytes[j] {
                b'>' => {
                    j += 1;
                    break;
                }
                b'/' if bytes.get(j + 1) == Some(&b'>') => {
                    j += 2;
                    break;
                }
                c if c.is_ascii_whitespace() => j += 1,
                _ => {
                    let key_start = j;
                    while j < bytes.len()
                        && !matches!(bytes[j], b'=' | b'>' | b'/')
                        && !bytes[j].is_ascii_whitespace()
                    {
                        j += 1;
                    }
                    if j == key_start {
                        // A stray `/` or `=` outside a value: step over it.
                        j += 1;
                        continue;
                    }
                    let key = text[key_start..j].to_string();
                    while j < bytes.len() && (bytes[j].is_ascii_whitespace() || bytes[j] == b'=') {
                        j += 1;
                    }
                    let Some(&quote) = bytes.get(j).filter(|q| matches!(q, b'"' | b'\'')) else {
                        continue;
                    };
                    let value_start = j + 1;
                    let value_end = text[value_start..]
                        .find(quote as char)
                        .map_or(bytes.len(), |p| value_start + p);
                    attributes.push((
                        key,
                        text[value_start..value_end].to_string(),
                        value_start as u32,
                        value_end as u32,
                    ));
                    j = value_end + 1;
                }
            }
        }
        found.push(Element {
            name,
            attributes,
            end: j as u32,
        });
        i = j;
    }
    found
}

/// Elements that map a class through their `name` attribute.
const MAPPED_CLASSES: [&str; 3] = ["entity", "embeddable", "mapped-superclass"];

/// Association elements, whose `field` names a property and whose
/// `target-entity` names the associated class.
const ASSOCIATIONS: [&str; 4] = ["one-to-one", "one-to-many", "many-to-one", "many-to-many"];

/// The elements an association holds before its `order-by`, which keep its target
/// entity for an `order-by-field`; the `cascade-*` elements count too.
const ASSOCIATION_PARTS: [&str; 8] = [
    "cache",
    "cascade",
    "join-table",
    "join-columns",
    "inverse-join-columns",
    "join-column",
    "order-by",
    "order-by-field",
];

/// Whether `path` is a Doctrine YAML mapping file.
pub(crate) fn is_doctrine_yaml(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|n| n.ends_with(".orm.yml") || n.ends_with(".orm.yaml"))
}

/// The association keys of a YAML mapping.
const YAML_ASSOCIATIONS: [&str; 4] = ["manyToOne", "oneToMany", "oneToOne", "manyToMany"];

/// The class or property the scalar under `offset` of a YAML mapping file names,
/// as `named_at` reads XML mappings.
pub(crate) fn yaml_named_at(root: &Yaml, offset: u32) -> Option<Named> {
    let (steps, Yaml::Scalar { value, .. }) = scalar_at(root, offset)? else {
        return None;
    };
    let itself = steps.last() == Some(&Step::KeyItself);
    let keys: Vec<&str> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Key(key) => Some(*key),
            _ => None,
        })
        .collect();
    let entity = |name: &str| name.trim_start_matches('\\').to_string();
    match (keys.as_slice(), itself) {
        ([], true) => Some(Named::Class(entity(value))),
        ([mapped, "repositoryClass"], false) => Some(Named::Class(qualify(mapped, value))),
        ([mapped, section], true)
            if matches!(*section, "fields" | "id" | "embedded")
                || YAML_ASSOCIATIONS.contains(section) =>
        {
            Some(Named::Property(entity(mapped), value.clone()))
        }
        ([mapped, section, _, "targetEntity"], false) if YAML_ASSOCIATIONS.contains(section) => {
            Some(Named::Class(qualify(mapped, value)))
        }
        ([mapped, "embedded", _, "class"], false) => Some(Named::Class(qualify(mapped, value))),
        ([mapped, section, property, "inversedBy" | "mappedBy"], false)
            if YAML_ASSOCIATIONS.contains(section) =>
        {
            let target = root
                .get(mapped)?
                .get(section)?
                .get(property)?
                .get("targetEntity")
                .and_then(Yaml::text)?;
            Some(Named::Property(qualify(mapped, target), value.clone()))
        }
        _ => None,
    }
}

/// What the attribute value under `offset` of a mapping file names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Named {
    Class(String),
    /// A property of the class.
    Property(String, String),
}

/// The class or property the attribute value under `offset` names, from the
/// element it belongs to and the class the enclosing element maps.
pub(crate) fn named_at(text: &str, offset: u32) -> Option<Named> {
    let mut mapped: Option<String> = None;
    // The target entity of the association the elements read are inside.
    let mut association: Option<String> = None;
    for element in elements(text) {
        let name = element.name.as_str();
        if MAPPED_CLASSES.contains(&name) {
            mapped = element
                .attribute("name")
                .map(|n| n.trim_start_matches('\\').to_string());
        }
        if ASSOCIATIONS.contains(&name) {
            association = element
                .attribute("target-entity")
                .zip(mapped.as_deref())
                .map(|(target, mapped)| qualify(mapped, target));
        } else if !ASSOCIATION_PARTS.contains(&name) && !name.starts_with("cascade") {
            association = None;
        }
        let Some((key, value, ..)) = element
            .attributes
            .iter()
            .find(|(_, _, start, end)| *start <= offset && offset <= *end)
        else {
            continue;
        };
        let class = |written: &str| mapped.as_deref().map(|m| qualify(m, written));
        return match key.as_str() {
            "name" if MAPPED_CLASSES.contains(&name) => {
                Some(Named::Class(value.trim_start_matches('\\').to_string()))
            }
            "target-entity" | "repository-class" => class(value).map(Named::Class),
            "class"
                if matches!(
                    name,
                    "embedded" | "discriminator-mapping" | "entity-listener"
                ) =>
            {
                class(value).map(Named::Class)
            }
            // Doctrine reads a custom generator's class as written.
            "class" if name == "custom-id-generator" => {
                Some(Named::Class(value.trim_start_matches('\\').to_string()))
            }
            "name" if name == "order-by-field" => {
                Some(Named::Property(association.clone()?, value.clone()))
            }
            // Doctrine reads `enum-type` as written, without the mapped namespace.
            "enum-type" if matches!(name, "field" | "id") => {
                Some(Named::Class(value.trim_start_matches('\\').to_string()))
            }
            "name" if matches!(name, "field" | "id" | "embedded") => {
                Some(Named::Property(mapped.clone()?, value.clone()))
            }
            "field" if ASSOCIATIONS.contains(&name) => {
                Some(Named::Property(mapped.clone()?, value.clone()))
            }
            "mapped-by" | "inversed-by" => {
                let target = class(element.attribute("target-entity")?)?;
                Some(Named::Property(target, value.clone()))
            }
            _ => None,
        };
    }
    None
}

/// What the mappings declare: each entity's repository class, and each mapped
/// superclass's, which its entities without their own take.
#[derive(Default)]
pub(crate) struct Mappings {
    /// Each mapped entity, with the repository it declares.
    entities: HashMap<String, Option<String>>,
    /// Each mapped superclass, with the repository it declares.
    superclasses: HashMap<String, Option<String>>,
    /// The namespace each entity alias stands for in `Alias:Entity`.
    namespaces: HashMap<String, String>,
}

/// The namespaces entity aliases stand for: each mapping of `doctrine.orm.mappings`
/// or `doctrine.orm.entity_managers.<name>.mappings` with a `prefix`, under its
/// `alias` or, as DoctrineBundle does, its name. An alias given different prefixes
/// is left out.
fn entity_namespaces(files: &[PathBuf], config_files: &ConfigFiles) -> HashMap<String, String> {
    let mut found: HashMap<String, Option<String>> = HashMap::new();
    for file in files {
        let Some(parsed) = config_files.get(file) else {
            continue;
        };
        let Some(root) = &parsed.yaml else {
            continue;
        };
        for block in top_blocks(root) {
            let Some(orm) = block
                .get("doctrine")
                .and_then(|doctrine| doctrine.get("orm"))
            else {
                continue;
            };
            let mut managers = vec![orm];
            if let Some(Yaml::Map(entries)) = orm.get("entity_managers") {
                managers.extend(entries.iter().map(|(_, manager)| manager));
            }
            for manager in managers {
                let Some(Yaml::Map(mappings)) = manager.get("mappings") else {
                    continue;
                };
                for (name, mapping) in mappings {
                    let (Some(name), Some(prefix)) =
                        (name.text(), mapping.get("prefix").and_then(Yaml::text))
                    else {
                        continue;
                    };
                    let alias = mapping.get("alias").and_then(Yaml::text).unwrap_or(name);
                    let prefix = prefix.trim_matches('\\').to_string();
                    found
                        .entry(alias.to_string())
                        .and_modify(|known| {
                            if known.as_ref() != Some(&prefix) {
                                *known = None;
                            }
                        })
                        .or_insert_with(|| Some(prefix));
                }
            }
        }
    }
    found
        .into_iter()
        .filter_map(|(alias, prefix)| Some((alias, prefix?)))
        .collect()
}

/// Records the mapped class `class`, keeping a repository it declares anywhere.
fn record(found: &mut HashMap<String, Option<String>>, class: &str, repository: Option<String>) {
    let known = found.entry(class.to_ascii_lowercase()).or_default();
    if repository.is_some() {
        *known = repository;
    }
}

impl Mappings {
    /// The `repository-class` of every entity and mapped superclass in the project's
    /// `*.orm.xml` files, then the repository of the `#[ORM\Entity]`,
    /// `@ORM\Entity` and `#[ORM\MappedSuperclass]` mappings of project classes, and
    /// the entity aliases the configuration files `config` define.
    pub(crate) fn read(
        index: &Index,
        filter: &PathFilter,
        files: &[PathBuf],
        config: &[PathBuf],
        config_files: &ConfigFiles,
    ) -> Mappings {
        let mut mappings = Mappings {
            namespaces: entity_namespaces(config, config_files),
            ..Mappings::default()
        };
        for file in files {
            let Some(parsed) = config_files.get(file) else {
                continue;
            };
            if is_doctrine_yaml(file) {
                // Each top-level key maps a class, its `type` saying how.
                let Some(Yaml::Map(entries)) = &parsed.yaml else {
                    continue;
                };
                for (key, mapping) in entries {
                    let Some(class) = key.text() else {
                        continue;
                    };
                    let found = match mapping.get("type").and_then(Yaml::text) {
                        Some("entity") => &mut mappings.entities,
                        Some("mappedSuperclass") => &mut mappings.superclasses,
                        _ => continue,
                    };
                    let class = class.trim_start_matches('\\');
                    let repository = mapping
                        .get("repositoryClass")
                        .and_then(Yaml::text)
                        .map(|repository| qualify(class, repository));
                    record(found, class, repository);
                }
                continue;
            }
            for element in elements(&parsed.text) {
                let found = match element.name.as_str() {
                    "entity" => &mut mappings.entities,
                    "mapped-superclass" => &mut mappings.superclasses,
                    _ => continue,
                };
                if let Some(entity) = element.attribute("name") {
                    let entity = entity.trim_start_matches('\\');
                    let repository = element
                        .attribute("repository-class")
                        .map(|repository| qualify(entity, repository));
                    record(found, entity, repository);
                }
            }
        }
        for (path, declaration) in index.declarations() {
            if declaration.kind != SymbolKind::Class || !filter.in_scope(path) {
                continue;
            }
            let Some(shape) = &declaration.class else {
                continue;
            };
            // `#[ORM\Entity]`, `@ORM\Entity` and `#[ORM\MappedSuperclass]`, the
            // repository named or first.
            for metadata in &shape.metadata {
                let found = if metadata.name.eq_ignore_ascii_case(ENTITY_ATTRIBUTE) {
                    &mut mappings.entities
                } else if metadata
                    .name
                    .eq_ignore_ascii_case(MAPPED_SUPERCLASS_ATTRIBUTE)
                {
                    &mut mappings.superclasses
                } else {
                    continue;
                };
                let first = metadata
                    .arguments
                    .first()
                    .filter(|(key, _)| key.is_none())
                    .map(|(_, value)| value);
                let repository = match metadata.argument("repositoryClass").or(first) {
                    Some(MetaValue::Class(class)) => Some(class.clone()),
                    Some(MetaValue::String(class)) => Some(qualify(&declaration.fqn, class)),
                    _ => None,
                };
                record(found, &declaration.fqn, repository);
            }
        }
        mappings
    }

    /// The entity `name` stands for: for `Alias:Entity`, the entity in the alias's
    /// namespace, none for an unknown alias; any other name itself.
    pub(crate) fn entity_named(&self, name: &str) -> Option<String> {
        match name.split_once(':') {
            Some((alias, entity)) => Some(format!("{}\\{entity}", self.namespaces.get(alias)?)),
            None => Some(name.to_string()),
        }
    }

    /// The repository of `entity`: its own, else, as `ClassMetadataFactory` does,
    /// the one of its nearest mapped ancestor when that is a mapped superclass;
    /// unmapped classes in between are skipped.
    pub(crate) fn repository_for(&self, index: &Index, entity: &str) -> Option<&str> {
        if let Some(own) = self.repository_of(entity) {
            return Some(own);
        }
        let mut next = index
            .classes_named(entity)
            .first()
            .and_then(|found| found.declaration.class.as_ref()?.parent.clone());
        for _ in 0..16 {
            let parent = next.take()?;
            let key = parent.to_ascii_lowercase();
            if let Some(repository) = self.superclasses.get(&key) {
                return repository.as_deref();
            }
            if self.entities.contains_key(&key) {
                return None;
            }
            next = index
                .classes_named(&parent)
                .first()
                .and_then(|found| found.declaration.class.as_ref()?.parent.clone());
        }
        None
    }

    pub(crate) fn repository_of(&self, entity: &str) -> Option<&str> {
        self.entities
            .get(&entity.trim_start_matches('\\').to_ascii_lowercase())?
            .as_deref()
    }
}

/// A class name written in the mapping of `mapped`: a name without a namespace is
/// in `mapped`'s namespace, as Doctrine resolves it; one with a backslash, even a
/// leading one alone, is already qualified.
pub(crate) fn qualify(mapped: &str, name: &str) -> String {
    if name.contains('\\') {
        return name.trim_start_matches('\\').to_string();
    }
    match mapped.rsplit_once('\\') {
        Some((namespace, _)) => format!("{namespace}\\{name}"),
        None => name.to_string(),
    }
}

/// Every `*.orm.xml` below `dir` the index records, outside the top `var`
/// directory and nested projects, in path order.
pub(crate) fn mapping_files(index: &Index, dir: &Path, filter: &PathFilter) -> Vec<PathBuf> {
    index
        .framework_files_under(dir)
        .filter(|path| (is_doctrine_xml(path) || is_doctrine_yaml(path)) && filter.in_scope(path))
        .filter(|path| {
            path.strip_prefix(dir)
                .is_ok_and(|relative| !relative.starts_with("var"))
        })
        .map(Path::to_path_buf)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Named, elements, named_at, qualify};

    #[test]
    fn reads_elements_and_their_attributes() {
        let text = "<?xml version=\"1.0\"?>\n<!-- <entity name=\"No\"/> -->\n<doctrine-mapping>\n  <entity name=\"A\\B\" table='t'>\n    <field name=\"x\"/>\n  </entity>\n</doctrine-mapping>\n";
        let found = elements(text);
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["doctrine-mapping", "entity", "field"]);
        let entity = &found[1];
        assert_eq!(entity.attribute("name"), Some("A\\B"));
        assert_eq!(entity.attribute("table"), Some("t"));
        let (_, value, start, end) = &entity.attributes[0];
        assert_eq!(&text[*start as usize..*end as usize], value);
    }

    /// Runs `elements` on `text` in a thread and fails when it does not finish.
    fn finishes(text: &'static str) -> Vec<String> {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(elements(text).into_iter().map(|e| e.name).collect());
        });
        receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap_or_else(|_| panic!("elements did not finish on {text:?}"))
    }

    #[test]
    fn malformed_tags_end() {
        finishes("<field name=\"x\" /\n<id name=\"y\"/>");
        finishes("<entity name=\"A\" / >");
        finishes("<entity name=\"A\" /");
        assert_eq!(
            finishes("<entity name=\"A\"><![CDATA[x > <field name=\"bogus\"/> <c / d>]]></entity>"),
            ["entity"]
        );
    }

    #[test]
    fn reads_more_mapping_attributes() {
        let text = r#"<doctrine-mapping>
  <entity name="App\Entity\Post">
    <discriminator-map><discriminator-mapping value="a" class="Article"/></discriminator-map>
    <entity-listeners><entity-listener class="PostListener"/></entity-listeners>
    <id name="id"><generator strategy="CUSTOM"/><custom-id-generator class="App\Id\IdGen"/></id>
    <one-to-many field="comments" target-entity="Comment" mapped-by="post">
      <order-by><order-by-field name="createdAt" direction="DESC"/></order-by>
    </one-to-many>
    <field name="title"/>
    <order-by-field name="stray"/>
  </entity>
</doctrine-mapping>"#;
        let at = |needle: &str| named_at(text, text.find(needle).unwrap() as u32 + 1);
        let class = |name: &str| Some(Named::Class(name.to_string()));
        assert_eq!(at("Article"), class("App\\Entity\\Article"));
        assert_eq!(at("PostListener"), class("App\\Entity\\PostListener"));
        assert_eq!(at("App\\Id"), class("App\\Id\\IdGen"));
        assert_eq!(
            at("createdAt"),
            Some(Named::Property(
                "App\\Entity\\Comment".to_string(),
                "createdAt".to_string()
            ))
        );
        assert_eq!(at("stray"), None);
    }

    #[test]
    fn order_by_fields_follow_cascades_caches_and_join_tables() {
        let text = r#"<doctrine-mapping>
  <entity name="App\Entity\Post">
    <one-to-many field="comments" target-entity="Comment" mapped-by="post">
      <cache usage="READ_ONLY"/>
      <cascade><cascade-persist/></cascade>
      <order-by><order-by-field name="createdAt"/></order-by>
    </one-to-many>
    <many-to-many field="tags" target-entity="Tag">
      <join-table name="post_tag">
        <join-columns><join-column name="post_id"/></join-columns>
        <inverse-join-columns><join-column name="tag_id"/></inverse-join-columns>
      </join-table>
      <order-by><order-by-field name="label"/></order-by>
    </many-to-many>
  </entity>
</doctrine-mapping>"#;
        let at = |needle: &str| named_at(text, text.find(needle).unwrap() as u32 + 1);
        let property =
            |class: &str, name: &str| Some(Named::Property(class.to_string(), name.to_string()));
        assert_eq!(
            at("createdAt"),
            property("App\\Entity\\Comment", "createdAt")
        );
        assert_eq!(at("label"), property("App\\Entity\\Tag", "label"));
    }

    #[test]
    fn short_names_take_the_mapped_namespace() {
        assert_eq!(
            qualify("App\\Entity\\Post", "Author"),
            "App\\Entity\\Author"
        );
        assert_eq!(
            qualify("App\\Entity\\Post", "App\\Other\\X"),
            "App\\Other\\X"
        );
        assert_eq!(
            qualify("App\\Entity\\Post", "\\App\\Other\\X"),
            "App\\Other\\X"
        );
    }
}
