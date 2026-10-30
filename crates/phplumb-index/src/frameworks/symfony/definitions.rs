//! Services and parameters defined outside YAML: PHP configurator files and XML
//! service files, the project's and the bundles'.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::RouteTarget;
use super::doctrine::elements;
use crate::index::Location;
use crate::infer::{configurator_definitions, parameter_setters};

/// The parameters bundle extensions set from code: names, the literal start and
/// end of names computed in between, and whether one sets a name computed
/// otherwise, which may be any. `complete` says whether every source of bundle
/// parameters was read: each bundle class, its extension and compiler passes, and
/// its PHP and XML configuration.
#[derive(Default)]
pub(crate) struct ExtensionParameters {
    pub names: HashSet<String>,
    pub patterns: Vec<(String, String)>,
    /// Names given as class constants, by class and constant, still to look up.
    pub constants: Vec<(String, String)>,
    pub any: bool,
    pub complete: bool,
}

impl ExtensionParameters {
    /// Adds the parameters the PHP file `source` sets: through `setParameter()`, or
    /// `set()` on a `parameters()` configurator.
    pub(crate) fn read_php(&mut self, source: &[u8]) {
        let setters = parameter_setters(source);
        self.names.extend(setters.names);
        for pattern in setters.patterns {
            if !self.patterns.contains(&pattern) {
                self.patterns.push(pattern);
            }
        }
        self.constants.extend(setters.constants);
        self.any |= setters.computed;
        let configured = configurator_definitions(source);
        self.names
            .extend(configured.parameters.into_iter().map(|(name, ..)| name));
        self.any |= configured.computed_parameters;
    }

    /// Whether an extension may set the parameter `name`.
    pub(crate) fn may_set(&self, name: &str) -> bool {
        self.any
            || self.names.contains(name)
            || self.patterns.iter().any(|(prefix, suffix)| {
                name.len() >= prefix.len() + suffix.len()
                    && name.starts_with(prefix.as_str())
                    && name.ends_with(suffix.as_str())
            })
    }
}

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

#[cfg(test)]
mod extension_tests {
    use super::ExtensionParameters;

    fn read(body: &str) -> ExtensionParameters {
        let mut found = ExtensionParameters::default();
        found.read_php(
            format!("<?php\nfunction load($container, $name, $busId) {{\n{body}\n}}\n").as_bytes(),
        );
        found
    }

    #[test]
    fn reads_the_parameters_extensions_set() {
        let literal = read("$container->setParameter('router.resource', 1);");
        assert!(literal.names.contains("router.resource"));
        assert!(literal.patterns.is_empty() && !literal.any);
        let suffix = read("$container->setParameter($busId.'.middleware', 1);");
        assert_eq!(
            suffix.patterns,
            [(String::new(), ".middleware".to_string())]
        );
        assert!(!suffix.any);
        let prefix = read("$container->setParameter('dama.'.Configuration::KEY, 1);");
        assert_eq!(prefix.patterns, [("dama.".to_string(), String::new())]);
        assert!(!prefix.any);
        let both = read("$container->setParameter('security.'.$name.'._indexed', 1);");
        assert_eq!(
            both.patterns,
            [("security.".to_string(), "._indexed".to_string())]
        );
        assert!(both.may_set("security.main._indexed"));
        assert!(!both.may_set("security._index"));
        let stof = || vec![("stof.listener.".to_string(), ".class".to_string())];
        let format = read("$container->setParameter(sprintf('stof.listener.%s.class', $name), 1);");
        assert_eq!(format.patterns, stof());
        assert!(!format.any);
        let interpolated = read("$container->setParameter(\"stof.listener.{$name}.class\", 1);");
        assert_eq!(interpolated.patterns, stof());
        assert!(!interpolated.any);
        assert!(read("$container->setParameter($name, 1);").any);
        let configured = read("$container->parameters()->set('acme.x', 1);");
        assert!(configured.names.contains("acme.x"));
    }
}
