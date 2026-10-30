//! Framework modules: what a framework adds on top of PHP, such as service
//! containers configured in YAML. Core navigation reaches them only through
//! `Framework` and `registered`.

mod ancestry;
mod attributes;
pub mod drupal;
mod project;
pub mod symfony;
pub(crate) mod yaml;

use std::any::Any;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::diagnostics::Diagnostic;
use crate::filter::PHP_EXTENSION;
use crate::index::{Index, Location};
use crate::metadata::{MetaValue, Metadata};
use crate::phpdoc::TypeExpr;
use crate::summary::{Declaration, FileSummary, Member, NameRef, RefKey};

pub use project::Project;

/// Whether a framework module runs: detected from the project, or forced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activation {
    #[default]
    Auto,
    On,
    Off,
}

/// One activation per framework module.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FrameworkSettings {
    pub drupal: Activation,
    pub symfony: Activation,
}

/// Whether `path` or one of its ancestors is in `paths`. A few paths are compared
/// directly, which costs less than hashing every ancestor.
pub(crate) fn under_any(path: &Path, paths: &HashSet<PathBuf>) -> bool {
    if paths.len() <= 16 {
        paths.iter().any(|known| path.starts_with(known))
    } else {
        path.ancestors().any(|ancestor| paths.contains(ancestor))
    }
}

/// Whether the summary mentions a member named one of `names`.
pub(crate) fn mentions_member(summary: &FileSummary, names: &[&str]) -> bool {
    names.iter().any(|name| {
        let key = RefKey::Member(name.to_string()).hash();
        summary.ref_keys.binary_search(&key).is_ok()
    })
}

/// Whether the summary mentions one of `classes`.
pub(crate) fn mentions(summary: &FileSummary, classes: &[&str]) -> bool {
    classes.iter().any(|class| {
        let key = RefKey::Class(class.to_string()).hash();
        summary.ref_keys.binary_search(&key).is_ok()
    })
}

/// What a framework sees of a call: what it targets and its literal arguments.
pub struct FrameworkCall<'a> {
    /// `Class::method` for every declaration the call resolves to, or the function.
    pub targets: &'a [CallTarget],
    /// Positional arguments that are string literals or `X::class`, by position.
    pub strings: &'a [Option<String>],
    /// The receiver's type, for method calls.
    pub receiver: Option<&'a TypeExpr>,
}

/// What a method call is made on, when it is a plain name: `$this->name` or
/// `$name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverName {
    Property(String),
    Variable(String),
}

/// What a framework sees of a call's arguments, receiver and place.
pub struct CallArguments<'a> {
    /// The type of each argument, by parameter position.
    pub types: &'a [TypeExpr],
    /// For each argument written `new C(x)` or `C::wrap(x)`, the type of `x`, by
    /// parameter position.
    pub wrapped: &'a [Option<TypeExpr>],
    /// What the call is made on, when it is a plain name.
    pub receiver: Option<&'a ReceiverName>,
    /// `Class::method` or the function the call is in.
    pub caller: Option<&'a str>,
}

/// The declaration a definition request is on: its name, or one of its attributes.
pub enum FrameworkDeclaration<'a> {
    /// A function, with every declaration of its file.
    Function {
        path: &'a Path,
        declaration: &'a Declaration,
        file: &'a [Declaration],
    },
    /// A method, by its name or one of its attributes.
    Method {
        path: &'a Path,
        class: &'a Declaration,
        member: &'a Member,
    },
    /// One of the class's own attributes.
    Class {
        path: &'a Path,
        class: &'a Declaration,
        metadata: &'a Metadata,
    },
}

/// A string literal under the cursor that is an argument of a PHP attribute, on a
/// class, function, method, property or parameter.
pub struct AttributeString {
    /// The attribute's fully qualified class name.
    pub attribute: String,
    /// The argument's name, when it is given by name.
    pub key: Option<String>,
    /// The argument's position among the attribute's arguments.
    pub position: usize,
    pub value: String,
    /// Where the value starts, inside the quotes.
    pub start: u32,
    /// The class whose declaration, or one of whose members, carries the attribute.
    pub holder: Option<String>,
    /// The declared type of the property the attribute is on.
    pub holder_type: Option<TypeExpr>,
    /// All the attribute's arguments.
    pub arguments: Vec<(Option<String>, MetaValue)>,
}

/// A method call found in the index, with its literal arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundCall {
    /// The method name at the call.
    pub location: Location,
    /// `Class::method` for every declaration the call resolves to.
    pub targets: Vec<CallTarget>,
    /// Positional arguments that are string literals or `X::class`, by position.
    pub strings: Vec<Option<String>>,
    /// The string literal elements of positional array arguments, by position.
    pub lists: Vec<Vec<String>>,
    /// The text shape of each positional argument, by position: a string's text
    /// with `*` for each part that is not literal, one per element of an array, or
    /// `*` for any other value.
    pub shapes: Vec<Vec<String>>,
    /// The byte range of each positional argument that is a string literal (inside
    /// its quotes) or `X::class`, by position.
    pub ranges: Vec<Option<(u32, u32)>>,
    /// The receiver's type; `Other` when unknown.
    pub receiver: TypeExpr,
    /// The type of each argument by the position of the parameter it fills, up to
    /// the first spread one; `Other` when unknown.
    pub arguments: Vec<TypeExpr>,
    /// For each argument written `new C(x, ...)` or `C::wrap(x, ...)`, the type of
    /// `x`, by position.
    pub wrapped: Vec<Option<TypeExpr>>,
    /// What the call is made on, when it is a plain name.
    pub receiver_name: Option<ReceiverName>,
    /// The method or function holding the call: `Class::method` with the declaring
    /// class's fully qualified name, or the function's fully qualified name.
    pub caller: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallTarget {
    Method { class: String, name: String },
    Function(String),
}

pub trait Framework: Send + Sync {
    /// The name shown in the log.
    fn name(&self) -> &'static str;
    /// The Composer package whose presence in a `composer.lock` marks a project.
    fn package(&self) -> &'static str;
    /// The part of the workspace the module reads and answers for.
    fn project(&self) -> &Project;
    fn set_project(&mut self, project: Project);
    /// Names of the directories holding code the framework installs rather than the
    /// project's own, nearest to project code first.
    fn dependency_directories(&self) -> &'static [&'static str];
    /// Extensions besides `php` of the PHP files the framework uses.
    fn php_extensions(&self) -> &'static [&'static str];
    /// Reads what the framework needs from the index and its own files, after the
    /// index is built. While `rebuild` and `update` run, the index has no active
    /// framework, so a framework cannot query the others.
    fn rebuild(&mut self, index: &Index);
    /// Applies changed and removed paths (files or directories, any extension),
    /// after the index applied them.
    fn update(&mut self, index: &Index, changed: &[PathBuf], removed: &[PathBuf]);
    /// Lowercased names of the methods whose calls `call_type` may type.
    fn typed_methods(&self) -> &'static [&'static str];
    /// Lowercased names of the methods whose calls `call_targets` may answer.
    fn targeted_methods(&self) -> &'static [&'static str];
    /// The type `call` returns when the framework knows better than its declaration.
    fn call_type(&self, index: &Index, call: &FrameworkCall<'_>) -> Option<TypeExpr>;
    /// Targets of the string literal at position `argument` of `call`, or of one of
    /// its elements when the argument is an array, each with the text expected at
    /// its range.
    fn string_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        argument: usize,
        value: &str,
    ) -> Vec<(Location, String)>;
    /// Targets of a definition request on the name of a method call, given its
    /// arguments, receiver and place, each with the text expected at its range;
    /// none to keep the method's declaration.
    fn call_targets(
        &self,
        index: &Index,
        call: &FrameworkCall<'_>,
        arguments: &CallArguments<'_>,
    ) -> Vec<(Location, String)>;
    /// Targets of a string literal under the cursor at `offset` that is an argument
    /// of an attribute, each with the text expected at its range.
    fn attribute_targets(
        &self,
        index: &Index,
        found: &AttributeString,
        offset: u32,
    ) -> Vec<(Location, String)>;
    /// Targets of a string literal that is the value of the array key `key`, each
    /// with the text expected at its range. `call` is the call whose positional
    /// argument at the given position holds the array, if any.
    fn keyed_string_targets(
        &self,
        index: &Index,
        key: &str,
        value: &str,
        call: Option<(&FrameworkCall<'_>, usize)>,
    ) -> Vec<(Location, String)>;
    /// Broken references the framework finds in the file at `path`, whose current
    /// content is `source`; `summary` is the summary of a PHP file, and none for the
    /// files only the frameworks read.
    fn diagnostics(
        &self,
        index: &Index,
        path: &Path,
        source: &[u8],
        summary: Option<&FileSummary>,
    ) -> Vec<Diagnostic>;
    /// Implementations the framework adds to the class-like `class`, each with the
    /// text expected at its range.
    fn class_implementations(&self, index: &Index, class: &str) -> Vec<(Location, String)>;
    /// The classes an unqualified annotation name no import resolves is, among the
    /// annotations of `class`, when the framework's discovery reads them; none when
    /// it does not.
    fn annotation_classes(&self, index: &Index, class: &str, name: &NameRef)
    -> Option<Vec<String>>;
    /// Targets at `offset` in a file the PHP engine does not read, whose current
    /// content is `source`, each with the text expected at its range.
    fn file_targets(
        &self,
        index: &Index,
        path: &Path,
        source: &[u8],
        offset: u32,
    ) -> Vec<(Location, String)>;
    /// Targets of a definition request on a declaration, each with the text expected
    /// at its range, when the framework knows what the declaration stands for.
    fn declaration_targets(
        &self,
        index: &Index,
        declaration: &FrameworkDeclaration<'_>,
    ) -> Vec<(Location, String)>;
    /// The module itself, for code that needs its own API.
    fn as_any(&self) -> &dyn Any;
}

/// Every framework the engine knows, each with its setting.
pub fn registered(settings: &FrameworkSettings) -> Vec<(Activation, Box<dyn Framework>)> {
    vec![
        (settings.drupal, Box::new(drupal::Drupal::default())),
        (settings.symfony, Box::new(symfony::Symfony::default())),
    ]
}

/// `php`, then the extensions `frameworks` add.
pub(crate) fn extensions(frameworks: &[Box<dyn Framework>]) -> Vec<&'static str> {
    std::iter::once(PHP_EXTENSION)
        .chain(
            frameworks
                .iter()
                .flat_map(|framework| framework.php_extensions().iter().copied()),
        )
        .collect()
}

/// The framework modules to run for a workspace, each with the directory of its
/// project.
#[derive(Clone, Debug)]
pub struct Detection {
    settings: FrameworkSettings,
    projects: Vec<(&'static str, PathBuf)>,
}

impl Detection {
    /// Extensions of the files read as PHP: `php`, then those the modules add.
    pub fn php_extensions(&self) -> Vec<&'static str> {
        extensions(&self.frameworks())
    }

    /// Fresh modules, each set to its project: the directory of its nearest
    /// `composer.lock`, or the root when forced on without one, without the projects
    /// of the other modules inside it.
    pub(crate) fn frameworks(&self) -> Vec<Box<dyn Framework>> {
        registered(&self.settings)
            .into_iter()
            .filter_map(|(_, mut framework)| {
                let (_, dir) = self
                    .projects
                    .iter()
                    .find(|(name, _)| *name == framework.name())?;
                let excluded = self
                    .projects
                    .iter()
                    .map(|(_, other)| other)
                    .filter(|other| *other != dir && other.starts_with(dir))
                    .cloned()
                    .collect();
                framework.set_project(Project {
                    dir: dir.clone(),
                    excluded,
                    linked: Vec::new(),
                });
                Some(framework)
            })
            .collect()
    }
}

/// Which framework modules run for the workspace at `root`. The `composer.lock`
/// files are looked for once, and not at all when every module is off.
pub fn detect(root: &Path, settings: &FrameworkSettings) -> Detection {
    detect_with(root, settings, project::locks)
}

pub(crate) fn detect_with(
    root: &Path,
    settings: &FrameworkSettings,
    find_locks: impl FnOnce(&Path) -> Vec<project::Lock>,
) -> Detection {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let modules = registered(settings);
    let locks = if modules
        .iter()
        .any(|(activation, _)| *activation != Activation::Off)
    {
        find_locks(&root)
    } else {
        Vec::new()
    };
    let projects = modules
        .into_iter()
        .filter_map(|(activation, framework)| {
            let lock = locks
                .iter()
                .find(|lock| lock.requires(framework.package()))
                .map(|lock| lock.dir.clone());
            let dir = match activation {
                Activation::Off => return None,
                Activation::On => lock.unwrap_or_else(|| root.clone()),
                Activation::Auto => lock?,
            };
            Some((framework.name(), dir))
        })
        .collect();
    Detection {
        settings: settings.clone(),
        projects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const BOTH_LOCK: &str =
        r#"{"packages": [{"name": "drupal/core"}, {"name": "symfony/framework-bundle"}]}"#;

    fn root_with_lock(lock: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("composer.lock"), lock).unwrap();
        (dir, root)
    }

    #[test]
    fn detection_skips_the_walk_when_every_module_is_off() {
        let (_dir, root) = root_with_lock(BOTH_LOCK);
        let settings = FrameworkSettings {
            drupal: Activation::Off,
            symfony: Activation::Off,
        };

        let detection = detect_with(&root, &settings, |_| panic!("walked for locks"));

        assert!(detection.frameworks().is_empty());
        assert_eq!(detection.php_extensions(), vec!["php"]);
    }

    #[test]
    fn under_any_matches_whole_components_for_few_and_many_paths() {
        let few: HashSet<PathBuf> = [PathBuf::from("/p/a")].into();
        let many: HashSet<PathBuf> = (0..40)
            .map(|i| PathBuf::from(format!("/q/{i}")))
            .chain([PathBuf::from("/p/a")])
            .collect();
        for paths in [&few, &many] {
            assert!(under_any(Path::new("/p/a"), paths));
            assert!(under_any(Path::new("/p/a/b.php"), paths));
            assert!(!under_any(Path::new("/p/ab.php"), paths));
            assert!(!under_any(Path::new("/p"), paths));
        }
        assert!(!under_any(Path::new("/p/a"), &HashSet::new()));
    }

    #[test]
    fn detection_finds_both_modules_in_one_lock() {
        let (_dir, root) = root_with_lock(BOTH_LOCK);

        let detection = detect(&root, &FrameworkSettings::default());

        let projects: Vec<(&str, PathBuf)> = detection
            .frameworks()
            .iter()
            .map(|f| (f.name(), f.project().dir.clone()))
            .collect();
        assert_eq!(
            projects,
            vec![("drupal", root.clone()), ("symfony", root.clone())]
        );
    }
}
