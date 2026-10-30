//! Services and parameters defined outside YAML: PHP configurator files and XML
//! service files, the project's and the bundles'.

use std::collections::HashMap;
use std::path::Path;

use super::RouteTarget;
use super::doctrine::elements;
use crate::index::Location;
use crate::infer::configurator_definitions;

/// The parameters, services and aliases some files define.
#[derive(Default)]
pub(crate) struct Definitions {
    pub parameters: HashMap<String, Vec<RouteTarget>>,
    pub services: HashMap<String, Vec<RouteTarget>>,
    /// Each alias with the services it stands for.
    pub aliases: HashMap<String, Vec<String>>,
}

impl Definitions {
    /// Adds what the PHP configurator file `path`, whose content is `source`,
    /// defines.
    pub(crate) fn read_php(&mut self, path: &Path, source: &[u8]) {
        let found = configurator_definitions(source);
        for (name, start, end) in found.parameters {
            add(&mut self.parameters, path, name, start, end);
        }
        for (id, start, end) in found.services {
            add(&mut self.services, path, id, start, end);
        }
        for (alias, service) in found.aliases {
            self.aliases.entry(alias).or_default().push(service);
        }
    }

    /// Adds what the XML service file `path`, whose content is `text`, defines:
    /// `<parameter key>`, `<service id>` and `<service id alias>`.
    pub(crate) fn read_xml(&mut self, path: &Path, text: &str) {
        for element in elements(text) {
            let named = |attribute: &str| {
                element
                    .attributes
                    .iter()
                    .find(|(key, ..)| key == attribute)
                    .map(|(_, value, start, end)| (value.clone(), *start, *end))
            };
            match element.name.as_str() {
                "parameter" => {
                    if let Some((key, start, end)) = named("key") {
                        add(&mut self.parameters, path, key, start, end);
                    }
                }
                "service" => {
                    let Some((id, start, end)) = named("id") else {
                        continue;
                    };
                    if let Some(alias) = element.attribute("alias") {
                        self.aliases
                            .entry(id.clone())
                            .or_default()
                            .push(alias.to_string());
                    }
                    add(&mut self.services, path, id, start, end);
                }
                _ => {}
            }
        }
    }
}

/// Adds the definition of `name` at `start..end` in `path` to `found`.
fn add(
    found: &mut HashMap<String, Vec<RouteTarget>>,
    path: &Path,
    name: String,
    start: u32,
    end: u32,
) {
    found.entry(name.clone()).or_default().push(RouteTarget {
        location: Location {
            path: path.to_path_buf(),
            start,
            end,
        },
        text: name,
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::Definitions;

    fn names(found: &std::collections::HashMap<String, Vec<super::RouteTarget>>) -> Vec<String> {
        let mut names: Vec<String> = found.keys().cloned().collect();
        names.sort();
        names
    }

    #[test]
    fn reads_php_configurators() {
        let source = br#"<?php
use App\Mailer;
return static function (ContainerConfigurator $container, string $name): void {
    $container->parameters()
        ->set('app.a', 1)
        ->set($name, 2);
    $parameters = $container->parameters();
    $parameters->set('app.b', 3);
    $services = $container->services();
    $services->set('app.mailer', Mailer::class)
        ->args([service('app.other'), param('app.c')])
        ->set(Mailer::class);
    $container->services()->alias('app.alias', 'app.mailer');
    $other->set('app.not', 4);
};
"#;
        let mut found = Definitions::default();
        found.read_php(Path::new("/c/services.php"), source);
        assert_eq!(names(&found.parameters), ["app.a", "app.b"]);
        assert_eq!(names(&found.services), ["App\\Mailer", "app.mailer"]);
        assert_eq!(found.aliases["app.alias"], ["app.mailer"]);
        let target = &found.parameters["app.a"][0];
        assert_eq!(
            &source[target.location.start as usize..target.location.end as usize],
            b"app.a"
        );
    }

    #[test]
    fn constants_are_no_names() {
        let source = br#"<?php
use Acme\Ext;
return static function ($container): void {
    $container->services()
        ->set(Ext::AUTHENTICATOR_ID, Ext::class)
        ->alias(Ext::class, Ext::CLIENT_ID);
};
"#;
        let mut found = Definitions::default();
        found.read_php(Path::new("/c/services.php"), source);
        assert!(found.services.is_empty());
        assert!(found.aliases.is_empty());
    }

    #[test]
    fn variables_take_their_latest_configurator() {
        let source = br#"<?php
return static function ($container): void {
    $s = $container->parameters();
    $s->set('app.p', 1);
    $s = $container->services();
    $s->set('app.svc', \stdClass::class);
};
"#;
        let mut found = Definitions::default();
        found.read_php(Path::new("/c/services.php"), source);
        assert_eq!(names(&found.parameters), ["app.p"]);
        assert_eq!(names(&found.services), ["app.svc"]);
    }

    #[test]
    fn reads_xml_service_files() {
        let text = r#"<container>
  <parameters><parameter key="app.x">1</parameter></parameters>
  <services>
    <service id="app.y" class="App\Y"><argument type="service" id="app.z"/></service>
    <service id="app.alias" alias="app.y"/>
  </services>
</container>"#;
        let mut found = Definitions::default();
        found.read_xml(Path::new("/c/services.xml"), text);
        assert_eq!(names(&found.parameters), ["app.x"]);
        assert_eq!(names(&found.services), ["app.alias", "app.y"]);
        assert_eq!(found.aliases["app.alias"], ["app.y"]);
    }
}
