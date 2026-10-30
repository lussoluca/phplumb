//! Doctrine mappings: the `*.orm.xml` files of the project and
//! `#[ORM\Entity(repositoryClass:)]` attributes, read for navigation and for the
//! repository `getRepository()` returns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::files::ConfigFiles;
use crate::filter::{PathFilter, is_doctrine_xml};
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
        found.push(Element { name, attributes });
        i = j;
    }
    found
}

/// Elements that map a class through their `name` attribute.
const MAPPED_CLASSES: [&str; 3] = ["entity", "embeddable", "mapped-superclass"];

/// Association elements, whose `field` names a property and whose
/// `target-entity` names the associated class.
const ASSOCIATIONS: [&str; 4] = ["one-to-one", "one-to-many", "many-to-one", "many-to-many"];

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
        } else if !matches!(name, "order-by" | "order-by-field") {
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
    repositories: HashMap<String, String>,
    superclasses: HashMap<String, String>,
}

impl Mappings {
    /// The `repository-class` of every entity and mapped superclass in the project's
    /// `*.orm.xml` files, then the repository of the `#[ORM\Entity]`,
    /// `@ORM\Entity` and `#[ORM\MappedSuperclass]` mappings of project classes.
    pub(crate) fn read(
        index: &Index,
        filter: &PathFilter,
        files: &[PathBuf],
        config_files: &ConfigFiles,
    ) -> Mappings {
        let mut mappings = Mappings::default();
        for file in files {
            let Some(parsed) = config_files.get(file) else {
                continue;
            };
            for element in elements(&parsed.text) {
                let found = match element.name.as_str() {
                    "entity" => &mut mappings.repositories,
                    "mapped-superclass" => &mut mappings.superclasses,
                    _ => continue,
                };
                if let (Some(entity), Some(repository)) = (
                    element.attribute("name"),
                    element.attribute("repository-class"),
                ) {
                    let entity = entity.trim_start_matches('\\');
                    found.insert(entity.to_ascii_lowercase(), qualify(entity, repository));
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
                    &mut mappings.repositories
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
                    Some(MetaValue::Class(class)) => class.clone(),
                    Some(MetaValue::String(class)) => qualify(&declaration.fqn, class),
                    _ => continue,
                };
                found.insert(declaration.fqn.to_ascii_lowercase(), repository);
            }
        }
        mappings
    }

    /// The repository of `entity`: its own, else the nearest mapped superclass
    /// ancestor's.
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
            if let Some(repository) = self.superclasses.get(&parent.to_ascii_lowercase()) {
                return Some(repository);
            }
            next = index
                .classes_named(&parent)
                .first()
                .and_then(|found| found.declaration.class.as_ref()?.parent.clone());
        }
        None
    }

    pub(crate) fn repository_of(&self, entity: &str) -> Option<&str> {
        self.repositories
            .get(&entity.trim_start_matches('\\').to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// A class name written in the mapping of `mapped`: a name without a namespace is
/// in `mapped`'s namespace, as Doctrine resolves it.
pub(crate) fn qualify(mapped: &str, name: &str) -> String {
    let name = name.trim_start_matches('\\');
    if name.contains('\\') {
        return name.to_string();
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
        .filter(|path| is_doctrine_xml(path) && filter.in_scope(path))
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
