//! Bundles: the classes `config/bundles.php` registers, and the directory each
//! reads its resources from.

use std::path::{Path, PathBuf};

use crate::index::Index;
use crate::summary::{MemberKind, NameKind, summarize};

/// Symfony's bundle classes, whose `getPath()` is the class's directory, or its
/// parent for an `AbstractBundle`.
const BUNDLE: &str = "Symfony\\Component\\HttpKernel\\Bundle\\Bundle";
const ABSTRACT_BUNDLES: [&str; 2] = [
    "Symfony\\Component\\HttpKernel\\Bundle\\AbstractBundle",
    "Symfony\\Component\\DependencyInjection\\Kernel\\AbstractBundle",
];

/// Each bundle `config/bundles.php` in `config` registers, in order: its name, the
/// short name of its class, and its directory as `getPath()` gives it, the class's
/// directory when that `getPath()` is `Bundle`'s, or its parent when it is an
/// `AbstractBundle`'s. The directory is none when the module cannot tell.
pub(crate) fn bundles(index: &Index, config: &Path) -> Vec<(String, Option<PathBuf>)> {
    let mut found: Vec<(String, Option<PathBuf>)> = Vec::new();
    for class in bundle_classes(config) {
        let short = class.rsplit('\\').next().unwrap_or(&class);
        if found.iter().any(|(known, _)| known == short) {
            continue;
        }
        found.push((short.to_string(), directory(index, &class)));
    }
    found
}

/// The classes `config/bundles.php` in `config` names, in order.
pub(crate) fn bundle_classes(config: &Path) -> Vec<String> {
    let Ok(source) = std::fs::read(config.join("bundles.php")) else {
        return Vec::new();
    };
    summarize(&source)
        .names
        .into_iter()
        .filter(|n| n.kind == NameKind::ClassLike)
        .map(|n| n.fqn)
        .collect()
}

/// The directory of the bundle `name` registered in `config/bundles.php`.
pub(crate) fn bundle_dir(index: &Index, config: &Path, name: &str) -> Option<PathBuf> {
    bundles(index, config)
        .into_iter()
        .find(|(known, _)| known == name)?
        .1
}

/// Whether the bundle class `fqn`, or one of its parents below Symfony's bundle
/// classes, declares or writes the `path` property `getPath()` returns.
fn sets_path(index: &Index, fqn: &str) -> bool {
    let mut next = Some(fqn.to_string());
    let mut seen = 0;
    while let Some(class) = next.take() {
        seen += 1;
        let symfony = std::iter::once(BUNDLE)
            .chain(ABSTRACT_BUNDLES)
            .any(|known| class.eq_ignore_ascii_case(known));
        if symfony || seen > 16 {
            return false;
        }
        let Some(found) = index.classes_named(&class).into_iter().next() else {
            return false;
        };
        let Some(shape) = &found.declaration.class else {
            return false;
        };
        let declares = shape
            .members
            .iter()
            .any(|m| m.kind == MemberKind::Property && m.name.eq_ignore_ascii_case("path"));
        let text = std::fs::read(found.path).unwrap_or_default();
        let (start, end) = found.declaration.range;
        let body = text.get(start as usize..end as usize).unwrap_or_default();
        let writes = body.windows(b"->path".len()).any(|w| w == b"->path");
        if declares || writes {
            return true;
        }
        next = shape.parent.clone();
    }
    false
}

/// The directory of the bundle class `fqn`; none when the bundle sets its own path,
/// by its `getPath()` or by its `path` property.
fn directory(index: &Index, fqn: &str) -> Option<PathBuf> {
    let class = index.classes_named(fqn).into_iter().next()?;
    if sets_path(index, fqn) {
        return None;
    }
    // The nearest `getPath()` decides: Symfony's own, or one the bundle sets.
    let (owner, _) = index
        .find_member(fqn, MemberKind::Method, "getPath")
        .into_iter()
        .next()?;
    let owner = &owner.declaration.fqn;
    let dir = class.path.parent()?;
    if owner.eq_ignore_ascii_case(BUNDLE) {
        Some(dir.to_path_buf())
    } else if ABSTRACT_BUNDLES
        .iter()
        .any(|known| owner.eq_ignore_ascii_case(known))
    {
        dir.parent().map(Path::to_path_buf)
    } else {
        None
    }
}
