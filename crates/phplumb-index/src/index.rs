//! Workspace-wide symbol table and the definition query.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Instant, UNIX_EPOCH};

use rayon::prelude::*;

use crate::builtins;
use crate::cache::{self, CachedFile};
use crate::calls::{self, CallItem, CallKind, Calls};
use crate::diagnostics::{self, Diagnostic};
use crate::discover::{self, DiscoverOptions, discover_in, discover_with};
use crate::filter::{PHP_EXTENSION, PathFilter, is_framework_file};
use crate::frameworks::{
    self, Detection, FoundCall, Framework, FrameworkCall, FrameworkDeclaration, Project,
};
use crate::hover::{self, Hover};
use crate::infer::{self, Assigning, PropertyTypes, Receiver};
use crate::members;
use crate::metadata::Metadata;
use crate::phpdoc::TypeExpr;
use crate::references::{self, Symbol};
use crate::summary::{
    ClassRange, ClassSel, Declaration, FileSummary, Member, MemberKind, NameKind, NameRef, RefKey,
    SymbolKind, summarize,
};
use crate::symbols::{self, SymbolMatch};

/// A byte range in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub start: u32,
    pub end: u32,
}

/// `reused` files came from the cache; `reparsed` files were read and parsed.
#[derive(Clone, Debug, Default)]
pub struct IndexStats {
    pub files: usize,
    pub declarations: usize,
    pub files_with_parse_errors: usize,
    pub reused: usize,
    pub reparsed: usize,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateStats {
    pub reparsed: usize,
    pub removed: usize,
    pub elapsed_ms: u128,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    file: u32,
    /// Index of the declaration in its file.
    decl: u32,
    start: u32,
    end: u32,
}

/// A class-like declaration found in the index.
#[derive(Clone, Copy, Debug)]
pub struct ClassRef<'a> {
    pub path: &'a Path,
    pub declaration: &'a Declaration,
}

/// Declarations of every PHP file under a root. Keys are lowercased fully qualified
/// names without a leading backslash, because PHP class and function names are
/// case-insensitive. Files are slots; a removed file leaves an empty slot. Each file
/// keeps its declarations, so their keys can be removed when it changes, and its size
/// and modification time, so a cache can tell whether it is still current.
pub struct Index {
    filter: PathFilter,
    /// Framework modules active for this project.
    frameworks: Vec<Box<dyn Framework>>,
    files: Vec<Option<CachedFile>>,
    ids: HashMap<PathBuf, u32>,
    classes: HashMap<String, Vec<Entry>>,
    functions: HashMap<String, Vec<Entry>>,
    constants: HashMap<String, Vec<Entry>>,
    /// Lowercased parent or interface name to the declarations that extend or
    /// implement it directly.
    subtype_table: HashMap<String, Vec<Entry>>,
    /// Reference key hash to the files that mention the key.
    candidates: HashMap<u64, Vec<u32>>,
    /// How many declarations, trait aliases included, declare each lowercased
    /// method name.
    method_names: HashMap<String, u32>,
    /// How many declarations live in each lowercased namespace or one below it.
    namespaces: HashMap<String, u32>,
    /// What each class's code assigns to its properties, by lowercased class name,
    /// computed on first use and emptied on every update.
    property_types: RwLock<HashMap<String, Arc<PropertyTypes>>>,
    /// The index of the phpstorm stubs, which declares PHP's built-in symbols.
    stubs: Option<Arc<Index>>,
}

impl Index {
    pub fn build(root: &Path, opts: &DiscoverOptions) -> anyhow::Result<(Index, IndexStats)> {
        let detection = frameworks::detect(root, &opts.frameworks);
        Self::build_with(root, opts, None, &detection)
    }

    /// Loads `cache` when it was written by this engine version for the same root and
    /// options, reuses entries whose file size and modification time are unchanged,
    /// and parses the rest. Any cache problem means a full build.
    pub fn build_with_cache(
        root: &Path,
        opts: &DiscoverOptions,
        cache: &Path,
    ) -> anyhow::Result<(Index, IndexStats)> {
        let detection = frameworks::detect(root, &opts.frameworks);
        Self::build_with(root, opts, Some(cache), &detection)
    }

    /// Builds the index with the framework modules `detection` found, from `cache`
    /// when there is one.
    pub fn build_with(
        root: &Path,
        opts: &DiscoverOptions,
        cache: Option<&Path>,
        detection: &Detection,
    ) -> anyhow::Result<(Index, IndexStats)> {
        let started = Instant::now();
        let active = detection.frameworks();
        let mut filter = PathFilter::for_frameworks(root, opts, &active)?;
        let mut cached = cache
            .map(|cache| cache::load(cache, &filter, cache::ENGINE))
            .unwrap_or_default();
        let paths = discover_with(&mut filter);

        let stamps: Vec<Option<(u64, u64)>> = paths.par_iter().map(|p| stamp(p)).collect();
        let mut files: Vec<Option<CachedFile>> = Vec::with_capacity(paths.len());
        let mut to_parse: Vec<(usize, PathBuf)> = Vec::new();
        for (i, (path, stamp)) in paths.into_iter().zip(stamps).enumerate() {
            match cached.remove(&path) {
                Some(hit) if stamp == Some((hit.size, hit.mtime_ns)) => files.push(Some(hit)),
                _ => {
                    files.push(None);
                    to_parse.push((i, path));
                }
            }
        }
        let mut stats = IndexStats {
            files: files.len(),
            reused: files.len() - to_parse.len(),
            reparsed: to_parse.len(),
            ..IndexStats::default()
        };
        let parse_paths: Vec<PathBuf> = to_parse.iter().map(|(_, p)| p.clone()).collect();
        for ((i, _), file) in to_parse.into_iter().zip(read_files(&parse_paths)) {
            files[i] = Some(file);
        }

        let mut index = Index {
            filter,
            frameworks: active,
            files: Vec::new(),
            ids: HashMap::new(),
            classes: HashMap::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
            subtype_table: HashMap::new(),
            candidates: HashMap::new(),
            method_names: HashMap::new(),
            namespaces: HashMap::new(),
            property_types: RwLock::new(HashMap::new()),
            stubs: opts.stubs.as_deref().and_then(crate::stubs::load),
        };
        for file in files.into_iter().flatten() {
            stats.files_with_parse_errors += usize::from(file.has_errors);
            stats.declarations += file.declarations.len();
            index.insert(file);
        }
        let mut frameworks = std::mem::take(&mut index.frameworks);
        for framework in &mut frameworks {
            framework.rebuild(&index);
        }
        index.frameworks = frameworks;
        index
            .property_types
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        stats.elapsed_ms = started.elapsed().as_millis();
        Ok((index, stats))
    }

    /// The index of the stub files below `dir`, each summarized as PHP
    /// `stubs::PHP_VERSION` has it, with no framework and no stubs of its own.
    pub(crate) fn build_stubs(dir: &Path) -> anyhow::Result<Index> {
        let opts = DiscoverOptions {
            stubs: None,
            ..DiscoverOptions::default()
        };
        let filter = PathFilter::for_frameworks(dir, &opts, &[])?;
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut pending = vec![filter.root().to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in std::fs::read_dir(&next)?.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|e| e == PHP_EXTENSION) {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        let mut index = Index {
            filter,
            frameworks: Vec::new(),
            files: Vec::new(),
            ids: HashMap::new(),
            classes: HashMap::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
            subtype_table: HashMap::new(),
            candidates: HashMap::new(),
            method_names: HashMap::new(),
            namespaces: HashMap::new(),
            property_types: RwLock::new(HashMap::new()),
            stubs: None,
        };
        for file in read_files_with(&paths, crate::stubs::summarize_stub) {
            index.insert(file);
        }
        // A name the stubs declare in several files, such as `Ds\Map` in `ds` and
        // `ds_v2`, keeps the declaration of the first file in path order.
        for table in [
            &mut index.classes,
            &mut index.functions,
            &mut index.constants,
        ] {
            for entries in table.values_mut() {
                entries.truncate(1);
            }
        }
        Ok(index)
    }

    /// The directory of the stubs this index reads built-in symbols from.
    pub fn stubs_dir(&self) -> Option<&Path> {
        self.stubs.as_ref().map(|stubs| stubs.filter.root())
    }

    /// Whether `path` is a stub file.
    pub fn is_stub(&self, path: &Path) -> bool {
        self.stubs_dir().is_some_and(|dir| path.starts_with(dir))
    }

    /// Writes the index to `cache` for a later `build_with_cache`.
    pub fn save_cache(&self, cache: &Path) -> anyhow::Result<()> {
        let files = self.files.iter().flatten().cloned().collect();
        cache::save(cache, &self.filter, cache::ENGINE, files)
    }

    /// Applies file system changes. `changed` holds created or modified files or
    /// directories; a directory is walked like a discovery. `removed` holds deleted
    /// files or directories. Changed files that no longer exist or that the filter
    /// rejects are treated as removed.
    pub fn update(&mut self, changed: &[PathBuf], removed: &[PathBuf]) -> UpdateStats {
        let started = Instant::now();
        let mut stats = UpdateStats::default();

        let deleted: Vec<PathBuf> = removed.iter().map(|p| self.filter.under_root(p)).collect();
        let mut rejected: Vec<PathBuf> = Vec::new();
        let mut fresh: Vec<PathBuf> = Vec::new();
        for path in changed {
            let path = self.filter.under_root(path);
            let chosen = discover::chosen_path(self.filter.root(), &path).map(|(_, chosen)| chosen);
            match chosen {
                Some(chosen) if chosen.is_dir() => {
                    if self.filter.accepts_dir(&chosen) {
                        fresh.extend(discover_in(&mut self.filter, &chosen));
                    }
                }
                Some(chosen) if self.filter.accepts(&chosen) => fresh.push(chosen),
                _ => rejected.push(path),
            }
        }

        // A path given twice must be read and inserted once.
        fresh.sort();
        fresh.dedup();
        stats.removed += self.remove_paths(&rejected, &deleted);
        for path in &fresh {
            if let Some(&id) = self.ids.get(path) {
                self.remove(id);
            }
        }
        stats.reparsed = fresh.len();
        for file in read_files(&fresh) {
            self.insert(file);
        }
        let changed: Vec<PathBuf> = changed.iter().map(|p| self.filter.under_root(p)).collect();
        let mut frameworks = std::mem::take(&mut self.frameworks);
        for framework in &mut frameworks {
            framework.update(self, &changed, &deleted);
        }
        self.frameworks = frameworks;
        self.property_types
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        stats.elapsed_ms = started.elapsed().as_millis();
        stats
    }

    /// Hover text for the word at `offset` in `path`, whose current content is
    /// `source`: the inferred type of a variable, or the signatures and docblock
    /// summaries of what the word refers to.
    pub fn hover(&mut self, path: &Path, source: &[u8], offset: u32) -> Option<Hover> {
        if is_framework_file(path) {
            return None;
        }
        hover::hover(self, path, source, offset)
    }

    /// References of what the cursor is on: every occurrence across the index that
    /// resolves to it, project code first. With `include_declaration`, the
    /// declarations themselves are listed too. Stub declarations have none.
    pub fn references(
        &mut self,
        path: &Path,
        source: &[u8],
        offset: u32,
        include_declaration: bool,
    ) -> Vec<Location> {
        let mut targets = self.targets(path, source, offset);
        targets.retain(|target| !self.is_stub(&target.path));
        let mut found: Vec<Location> = Vec::new();
        for target in &targets {
            if let Some(symbol) = self.symbol_at(target) {
                found.extend(references::scan(self, &symbol));
            }
        }
        if include_declaration {
            found.extend(targets);
        }
        found.sort_by(|a, b| {
            (self.rank(&a.path), &a.path, a.start).cmp(&(self.rank(&b.path), &b.path, b.start))
        });
        found.dedup();
        found
    }

    /// The rules deciding which files belong to the index.
    pub(crate) fn filter(&self) -> &PathFilter {
        &self.filter
    }

    /// Instance, null-safe and static calls to methods named one of `names`, in
    /// every candidate file, sorted by path and offset.
    pub fn calls_named(&self, names: &[&str]) -> Vec<FoundCall> {
        let mut files: Vec<PathBuf> = names
            .iter()
            .flat_map(|name| self.candidate_files(&RefKey::Member(name.to_string())))
            .map(Path::to_path_buf)
            .collect();
        files.sort();
        files.dedup();
        let mut found: Vec<FoundCall> = files
            .par_iter()
            .flat_map_iter(|path| {
                let Ok(source) = std::fs::read(path) else {
                    return Vec::new();
                };
                let summary = summarize(&source);
                infer::calls_named(self, path, &summary, &source, names)
            })
            .collect();
        found.sort_by(|a, b| {
            (&a.location.path, a.location.start).cmp(&(&b.location.path, b.location.start))
        });
        found
    }

    /// The classes an annotation name of `summary` may be: the class a qualified or
    /// imported name names; for an unimported name on a class whose annotations a
    /// framework's discovery reads, what that framework reads it as; otherwise the
    /// class of the file's namespace, as Doctrine reads it.
    pub(crate) fn annotation_classes(&self, summary: &FileSummary, name: &NameRef) -> Vec<String> {
        if name.qualified || name.imported {
            return vec![name.fqn.clone()];
        }
        let class = summary
            .declarations
            .iter()
            .find(|d| {
                d.class.as_ref().is_some_and(|shape| {
                    shape
                        .metadata
                        .iter()
                        .any(|m| m.annotation && m.span.0 <= name.start && name.start <= m.span.1)
                })
            })
            .map(|d| d.fqn.as_str());
        let discovered: Vec<Vec<String>> = class
            .map(|class| {
                self.frameworks
                    .iter()
                    .filter(|f| {
                        self.classes_named(class)
                            .iter()
                            .any(|c| f.project().contains(c.path))
                    })
                    .filter_map(|f| f.annotation_classes(self, class, name))
                    .collect()
            })
            .unwrap_or_default();
        if discovered.is_empty() {
            return if self.has_class(&name.fqn) {
                vec![name.fqn.clone()]
            } else {
                Vec::new()
            };
        }
        let mut found: Vec<String> = discovered.into_iter().flatten().collect();
        found.sort();
        found.dedup();
        found
    }

    /// Every indexed declaration with the path of its file.
    pub fn declarations(&self) -> impl Iterator<Item = (&Path, &Declaration)> {
        self.files
            .iter()
            .flatten()
            .flat_map(|file| file.declarations.iter().map(|d| (file.path.as_path(), d)))
    }

    /// The framework module of type `F`, when it is active.
    pub fn framework<F: Framework + 'static>(&self) -> Option<&F> {
        self.frameworks
            .iter()
            .find_map(|f| f.as_any().downcast_ref::<F>())
    }

    /// The framework modules active for this project.
    pub(crate) fn frameworks(&self) -> &[Box<dyn Framework>] {
        &self.frameworks
    }

    /// The active framework modules whose project holds `path`: the only ones asked
    /// about the file.
    pub(crate) fn frameworks_for<'a>(
        &'a self,
        path: &'a Path,
    ) -> impl Iterator<Item = &'a dyn Framework> + 'a {
        let path = self.filter.under_root(path);
        self.frameworks
            .iter()
            .filter(move |f| f.project().contains(&path))
            .map(|f| f.as_ref())
    }

    /// The project of the active framework module named `name`.
    pub fn framework_project(&self, name: &str) -> Option<&Project> {
        self.frameworks
            .iter()
            .find(|f| f.name() == name)
            .map(|f| f.project())
    }

    /// The names of the framework modules active for this project.
    pub fn active_frameworks(&self) -> Vec<&'static str> {
        self.frameworks.iter().map(|f| f.name()).collect()
    }

    /// How far `path` is from project code, for sorting: 0 for project code, then
    /// one step for each directory the active frameworks install code in, nearest
    /// first, then `vendor`. Only the part of the path below the root counts.
    pub(crate) fn rank(&self, path: &Path) -> u8 {
        let relative = path.strip_prefix(self.filter.root()).unwrap_or(path);
        let directories = || {
            self.frameworks
                .iter()
                .flat_map(|framework| framework.dependency_directories())
        };
        let vendor = directories().count() + 1;
        let rank = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .map(|name| match name {
                "vendor" => vendor,
                _ => directories()
                    .position(|directory| *directory == name)
                    .map_or(0, |i| i + 1),
            })
            .max()
            .unwrap_or(0);
        u8::try_from(rank).unwrap_or(u8::MAX)
    }

    /// Whether `path` is the project's own code: below the root, and outside
    /// `vendor` and the directories the active frameworks install code in.
    pub fn is_project_code(&self, path: &Path) -> bool {
        path.starts_with(self.filter.root()) && self.rank(path) == 0
    }

    /// Broken references in `source`, the content of the file at `path`, sorted by
    /// offset. Every rule must prove the target missing; uncertainty stays silent.
    pub fn diagnostics(&self, path: &Path, source: &[u8]) -> Vec<Diagnostic> {
        // Only the frameworks read YAML, Twig and Doctrine XML files.
        let mut found = if is_framework_file(path) {
            Vec::new()
        } else {
            diagnostics::find(self, path, source)
        };
        // `*.api.php` files document hooks with example code that never runs.
        let documentation = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".api.php"));
        if !documentation {
            found.extend(
                self.frameworks_for(path)
                    .flat_map(|f| f.diagnostics(self, path, source)),
            );
        }
        found.sort_by_key(|d| (d.start, d.end));
        found
    }

    /// Whether a class-like named `fqn` is indexed.
    pub(crate) fn has_class(&self, fqn: &str) -> bool {
        !self
            .entries(SymbolKind::Class, &fqn.to_ascii_lowercase())
            .1
            .is_empty()
    }

    /// Whether a function named `fqn` is indexed.
    pub(crate) fn has_function(&self, fqn: &str) -> bool {
        !lookup(&self.functions, &fqn.to_ascii_lowercase()).is_empty()
    }

    /// Whether any indexed class-like declares a method named `name`, directly or
    /// as a trait alias.
    pub(crate) fn has_method_named(&self, name: &str) -> bool {
        self.method_names.contains_key(&name.to_ascii_lowercase())
    }

    /// Whether the user excluded any paths from the index, so a name missing from it
    /// may still exist on disk.
    pub(crate) fn has_exclusions(&self) -> bool {
        self.filter.has_exclusions()
    }

    /// Whether any indexed declaration lives in `namespace` or below it. The global
    /// namespace is always known.
    pub(crate) fn knows_namespace(&self, namespace: &str) -> bool {
        namespace.is_empty()
            || self
                .namespaces
                .contains_key(&namespace.to_ascii_lowercase())
    }

    /// Whether a constant named `fqn` is indexed.
    pub(crate) fn has_constant(&self, fqn: &str) -> bool {
        !lookup(&self.constants, &symbol_key(SymbolKind::Constant, fqn)).is_empty()
    }

    /// Declarations and members matching `query`, best first, at most `limit`.
    pub fn workspace_symbols(&self, query: &str, limit: usize) -> Vec<SymbolMatch> {
        let files = self.files.iter().flatten();
        symbols::search(
            files.map(|f| (f.path.as_path(), f.declarations.as_slice())),
            query,
            limit,
            |path| self.rank(path),
        )
    }

    /// The functions and methods the cursor resolves to, as call hierarchy items.
    pub fn call_items(&mut self, path: &Path, source: &[u8], offset: u32) -> Vec<CallItem> {
        self.targets(path, source, offset)
            .iter()
            .filter_map(|target| self.call_item_at(target))
            .collect()
    }

    /// Callers of `item`, each with the call ranges inside it. Calls in top-level code
    /// belong to a file item.
    pub fn incoming_calls(&mut self, item: &CallItem) -> Calls {
        calls::incoming(self, item)
    }

    /// Functions and methods called inside `item`'s body, each with the call ranges.
    pub fn outgoing_calls(&mut self, item: &CallItem) -> Calls {
        calls::outgoing(self, item)
    }

    /// The function or method declared with its name at `target`.
    pub(crate) fn call_item_at(&self, target: &Location) -> Option<CallItem> {
        let file = self.files[*self.ids.get(&target.path)? as usize].as_ref()?;
        for declaration in &file.declarations {
            if declaration.kind == SymbolKind::Function && declaration.name_start == target.start {
                let (namespace, name) = calls::split(&declaration.fqn);
                return Some(CallItem {
                    name: name.to_string(),
                    kind: CallKind::Function,
                    detail: namespace.to_string(),
                    path: file.path.clone(),
                    range: declaration.range,
                    selection: (declaration.name_start, declaration.name_end),
                });
            }
            let method = declaration
                .class
                .iter()
                .flat_map(|shape| shape.members.iter())
                .find(|m| m.kind == MemberKind::Method && m.name_start == target.start);
            if let Some(method) = method {
                return Some(calls::method_item(&file.path, &declaration.fqn, method));
            }
        }
        None
    }

    /// The symbol declared at `target`: a class-like, function, constant or member.
    pub(crate) fn symbol_at(&self, target: &Location) -> Option<Symbol> {
        let file = self.files[*self.ids.get(&target.path)? as usize].as_ref()?;
        for declaration in &file.declarations {
            if declaration.name_start == target.start {
                return Some(match declaration.kind {
                    SymbolKind::Function => Symbol::Function(declaration.fqn.clone()),
                    SymbolKind::Constant => Symbol::Constant(declaration.fqn.clone()),
                    _ => Symbol::Class(declaration.fqn.clone()),
                });
            }
            let Some(member) = declaration
                .class
                .iter()
                .flat_map(|shape| shape.members.iter())
                .find(|m| m.name_start == target.start)
            else {
                continue;
            };
            let kind = match member.kind {
                MemberKind::EnumCase => MemberKind::Constant,
                kind => kind,
            };
            let name = member.name.as_str();
            // A trait member belongs to the classes using it.
            let owners: Vec<String> = if declaration.kind == SymbolKind::Trait {
                self.trait_users(&declaration.fqn)
                    .into_iter()
                    .filter(|user| self.resolves_to(user, kind, name, target))
                    .collect()
            } else {
                vec![declaration.fqn.clone()]
            };
            // A private member overrides and implements nothing.
            let mut family = if member.is_private {
                Vec::new()
            } else {
                self.family(&owners, kind, name, target)
            };
            // An abstract member has no body of its own: a call that resolves to one
            // of its implementations is a call to it.
            if member.is_abstract {
                for implementation in self.implementations_of(&owners, kind, name, target) {
                    if !family.iter().any(|(l, _)| *l == implementation.0) {
                        family.push(implementation);
                    }
                }
            }
            let constructed =
                if kind == MemberKind::Method && name.eq_ignore_ascii_case("__construct") {
                    owners
                        .iter()
                        .flat_map(|owner| {
                            let subtypes = self.all_subtypes(owner);
                            std::iter::once(owner.clone())
                                .chain(subtypes.into_iter().map(|s| s.declaration.fqn.clone()))
                        })
                        .filter(|class| self.resolves_to(class, kind, name, target))
                        .collect()
                } else {
                    Vec::new()
                };
            return Some(Symbol::Member {
                name: name.to_string(),
                kind,
                own: target.clone(),
                family,
                constructed,
            });
        }
        None
    }

    /// Every non-private declaration the supertypes of `owners` resolve the member
    /// to, other than `own`, labeled `Class::member`: calls typed as one of those
    /// supertypes may run `own`.
    fn family(
        &self,
        owners: &[String],
        kind: MemberKind,
        name: &str,
        own: &Location,
    ) -> Vec<(Location, String)> {
        let mut family: Vec<(Location, String)> = Vec::new();
        let mut seen: HashSet<String> = owners.iter().map(|o| o.to_ascii_lowercase()).collect();
        let mut queue: Vec<String> = owners.to_vec();
        while let Some(next) = queue.pop() {
            for supertype in self.supertypes(&next) {
                let fqn = supertype.declaration.fqn.clone();
                if !seen.insert(fqn.to_ascii_lowercase()) {
                    continue;
                }
                for (class, member) in self.find_member(&fqn, kind, name) {
                    let location = Location {
                        path: class.path.to_path_buf(),
                        start: member.name_start,
                        end: member.name_end,
                    };
                    if member.is_private
                        || location == *own
                        || family.iter().any(|(l, _)| *l == location)
                    {
                        continue;
                    }
                    let (_, short) = calls::split(&class.declaration.fqn);
                    family.push((location, format!("{short}::{}", member.name)));
                }
                queue.push(fqn);
            }
        }
        family
    }

    /// The declarations that concrete subtypes of `owners` resolve the member to, other
    /// than `own`, labeled `Class::member`.
    fn implementations_of(
        &self,
        owners: &[String],
        kind: MemberKind,
        name: &str,
        own: &Location,
    ) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = Vec::new();
        for owner in owners {
            for subtype in self.all_subtypes(owner) {
                let concrete = subtype
                    .declaration
                    .class
                    .as_ref()
                    .is_some_and(|s| !s.is_abstract)
                    && !matches!(
                        subtype.declaration.kind,
                        SymbolKind::Interface | SymbolKind::Trait
                    );
                if !concrete {
                    continue;
                }
                for (class, member) in self.find_member(&subtype.declaration.fqn, kind, name) {
                    let location = Location {
                        path: class.path.to_path_buf(),
                        start: member.name_start,
                        end: member.name_end,
                    };
                    if member.is_private
                        || location == *own
                        || found.iter().any(|(l, _)| *l == location)
                    {
                        continue;
                    }
                    let (_, short) = calls::split(&class.declaration.fqn);
                    found.push((location, format!("{short}::{}", member.name)));
                }
            }
        }
        found
    }

    /// Whether `class` resolves member `name` to the declaration at `own`.
    fn resolves_to(&self, class: &str, kind: MemberKind, name: &str, own: &Location) -> bool {
        self.find_member(class, kind, name)
            .iter()
            .any(|(c, m)| c.path == own.path && m.name_start == own.start)
    }

    /// Classes and enums using the trait `fqn`, directly or through other traits.
    fn trait_users(&self, fqn: &str) -> Vec<String> {
        let mut users = Vec::new();
        let mut seen: HashSet<String> = HashSet::from([fqn.to_ascii_lowercase()]);
        let mut queue: Vec<String> = vec![fqn.to_string()];
        while let Some(next) = queue.pop() {
            let declarations = self
                .files
                .iter()
                .flatten()
                .flat_map(|f| f.declarations.iter());
            for declaration in declarations {
                let uses = declaration.class.as_ref().is_some_and(|shape| {
                    shape.traits.iter().any(|t| t.eq_ignore_ascii_case(&next))
                });
                if !uses || !seen.insert(declaration.fqn.to_ascii_lowercase()) {
                    continue;
                }
                if declaration.kind == SymbolKind::Trait {
                    queue.push(declaration.fqn.clone());
                } else {
                    users.push(declaration.fqn.clone());
                }
            }
        }
        users
    }

    /// Whether the function name `name` reaches the function `fqn`, directly or
    /// through PHP's global fallback.
    pub(crate) fn function_matches(&self, name: &NameRef, fqn: &str) -> bool {
        if name.fqn.eq_ignore_ascii_case(fqn) {
            return true;
        }
        let fallback = !name.qualified && !name.imported && !fqn.contains('\\');
        fallback
            && name
                .fqn
                .rsplit_once('\\')
                .is_some_and(|(_, short)| short.eq_ignore_ascii_case(fqn))
            && lookup(&self.functions, &name.fqn.to_ascii_lowercase()).is_empty()
    }

    /// Whether the constant name `name` reaches the constant `fqn`, directly or
    /// through PHP's global fallback.
    pub(crate) fn constant_matches(&self, name: &NameRef, fqn: &str) -> bool {
        let wanted = symbol_key(SymbolKind::Constant, fqn);
        if symbol_key(SymbolKind::Constant, &name.fqn) == wanted {
            return true;
        }
        let fallback = !name.qualified && !name.imported && !fqn.contains('\\');
        fallback
            && name
                .fqn
                .rsplit_once('\\')
                .is_some_and(|(_, short)| short == fqn)
            && lookup(
                &self.constants,
                &symbol_key(SymbolKind::Constant, &name.fqn),
            )
            .is_empty()
    }

    /// Files that mention `key`, possibly with false positives, never false negatives.
    pub fn candidate_files(&self, key: &RefKey) -> Vec<&Path> {
        self.candidates
            .get(&key.hash())
            .into_iter()
            .flatten()
            .filter_map(|&id| self.files[id as usize].as_ref())
            .map(|file| file.path.as_path())
            .collect()
    }

    /// The class-like declared with its name starting at `start` in `path`.
    pub fn class_declared_at(&self, path: &Path, start: u32) -> Option<ClassRef<'_>> {
        let file = self.files[*self.ids.get(path)? as usize].as_ref()?;
        let declaration = file
            .declarations
            .iter()
            .find(|d| d.class.is_some() && d.name_start == start)?;
        Some(ClassRef {
            path: &file.path,
            declaration,
        })
    }

    /// Every class-like declaration named `fqn`, matched case-insensitively.
    pub fn classes_named(&self, fqn: &str) -> Vec<ClassRef<'_>> {
        let (holder, entries) = self.entries(SymbolKind::Class, &fqn.to_ascii_lowercase());
        holder.class_refs(entries)
    }

    /// The index that declares `key` of `kind`: the stubs for a name PHP itself
    /// declares, when they are loaded and project code does not declare it, and this
    /// index otherwise. A polyfill in `vendor` gives way to the stubs.
    fn holder(&self, kind: SymbolKind, key: &str) -> &Index {
        let built_in = match kind {
            SymbolKind::Function => builtins::is_function(key),
            SymbolKind::Constant => builtins::is_constant(key),
            _ => builtins::is_class(key),
        };
        match &self.stubs {
            Some(stubs) if built_in && !self.project_declares(kind, key) => stubs,
            _ => self,
        }
    }

    /// The lowercase global name of the PHP function a call to `name` reaches when
    /// no project file declares it: the name itself in the global namespace, or
    /// its last segment through PHP's global fallback for an unqualified, not
    /// imported name. Vendor polyfills give way, as they do for definitions.
    pub(crate) fn builtin_function(&self, name: &NameRef) -> Option<String> {
        let key = name.fqn.to_ascii_lowercase();
        let global = match key.rsplit_once('\\') {
            None => key,
            Some((_, global)) if !name.qualified && !name.imported => {
                if !lookup(&self.functions, &key).is_empty() {
                    return None;
                }
                global.to_string()
            }
            Some(_) => return None,
        };
        (!self.project_declares(SymbolKind::Function, &global)).then_some(global)
    }

    /// Whether project code declares `key` of `kind`.
    fn project_declares(&self, kind: SymbolKind, key: &str) -> bool {
        lookup(self.table(kind), key).iter().any(|entry| {
            self.files[entry.file as usize]
                .as_ref()
                .is_some_and(|file| self.is_project_code(&file.path))
        })
    }

    /// The holder of `key` of `kind` and the entries its table has for the key.
    fn entries(&self, kind: SymbolKind, key: &str) -> (&Index, &[Entry]) {
        let holder = self.holder(kind, key);
        (holder, lookup(holder.table(kind), key))
    }

    fn table(&self, kind: SymbolKind) -> &HashMap<String, Vec<Entry>> {
        match kind {
            SymbolKind::Function => &self.functions,
            SymbolKind::Constant => &self.constants,
            _ => &self.classes,
        }
    }

    /// Direct subtypes: classes extending `fqn`, classes and enums implementing it,
    /// interfaces extending it.
    pub fn subtypes(&self, fqn: &str) -> Vec<ClassRef<'_>> {
        self.class_refs(lookup(&self.subtype_table, &fqn.to_ascii_lowercase()))
    }

    /// Every subtype of `fqn`, transitively, each once.
    pub fn all_subtypes(&self, fqn: &str) -> Vec<ClassRef<'_>> {
        let mut seen: HashSet<String> = HashSet::from([fqn.to_ascii_lowercase()]);
        let mut queue: Vec<String> = vec![fqn.to_string()];
        let mut found = Vec::new();
        while let Some(next) = queue.pop() {
            for subtype in self.subtypes(&next) {
                if seen.insert(subtype.declaration.fqn.to_ascii_lowercase()) {
                    queue.push(subtype.declaration.fqn.clone());
                    found.push(subtype);
                }
            }
        }
        found
    }

    /// Direct supertypes: the parent and the interfaces of every declaration of `fqn`.
    pub fn supertypes(&self, fqn: &str) -> Vec<ClassRef<'_>> {
        let mut found: Vec<ClassRef<'_>> = Vec::new();
        for class in self.classes_named(fqn) {
            for supertype in supertype_names(class.declaration) {
                for candidate in self.classes_named(supertype) {
                    let duplicate = found.iter().any(|f| {
                        f.path == candidate.path
                            && f.declaration.name_start == candidate.declaration.name_start
                    });
                    if !duplicate {
                        found.push(candidate);
                    }
                }
            }
        }
        found
    }

    fn class_refs(&self, entries: &[Entry]) -> Vec<ClassRef<'_>> {
        entries
            .iter()
            .filter_map(|entry| {
                let file = self.files[entry.file as usize].as_ref()?;
                Some(ClassRef {
                    path: &file.path,
                    declaration: file.declarations.get(entry.decl as usize)?,
                })
            })
            .collect()
    }

    /// Every declaration of `name` of `kind` visible on `class`, following the
    /// member lookup order through traits, parents and interfaces.
    pub fn find_member(
        &self,
        class: &str,
        kind: MemberKind,
        name: &str,
    ) -> Vec<(ClassRef<'_>, &Member)> {
        members::find_member(self, class, kind, name)
    }

    /// The return types of the functions a call to `name` reaches, with PHP's global
    /// fallback for unqualified, non-imported names.
    pub(crate) fn function_returns(&self, name: &NameRef) -> Vec<TypeExpr> {
        let key = name.fqn.to_ascii_lowercase();
        let (mut holder, mut entries) = self.entries(SymbolKind::Function, &key);
        if entries.is_empty()
            && !name.qualified
            && !name.imported
            && let Some((_, global)) = key.rsplit_once('\\')
        {
            (holder, entries) = self.entries(SymbolKind::Function, global);
        }
        entries
            .iter()
            .filter_map(|entry| {
                let file = holder.files[entry.file as usize].as_ref()?;
                file.declarations.get(entry.decl as usize)?.returns.clone()
            })
            .collect()
    }

    /// `class` and its ancestors through `extends`, lowercased, each once.
    pub(crate) fn parent_chain(&self, class: &str) -> Vec<String> {
        let mut chain = vec![class.to_ascii_lowercase()];
        let mut next = 0;
        while next < chain.len() {
            let parents: Vec<String> = self
                .classes_named(&chain[next])
                .iter()
                .filter_map(|c| c.declaration.class.as_ref()?.parent.as_deref())
                .map(str::to_ascii_lowercase)
                .collect();
            for parent in parents {
                if !chain.contains(&parent) {
                    chain.push(parent);
                }
            }
            next += 1;
        }
        chain
    }

    /// Whether `class`, a lowercased class name, declares property `name` private.
    pub(crate) fn has_private_property(&self, class: &str, name: &str) -> bool {
        self.classes_named(class).iter().any(|c| {
            c.declaration.class.as_ref().is_some_and(|shape| {
                shape
                    .members
                    .iter()
                    .any(|m| m.is_private && members::matches(m, MemberKind::Property, name))
            })
        })
    }

    /// What the code of `class`, a lowercased class name, assigns to its
    /// properties. `outer` is the computation that asks, if any.
    pub(crate) fn property_types(
        &self,
        class: &str,
        outer: Option<&Assigning<'_>>,
    ) -> Arc<PropertyTypes> {
        if let Some(found) = self
            .property_types
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(class)
        {
            return found.clone();
        }
        let computed = Arc::new(infer::property_assignments(self, class, outer));
        self.property_types
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(class.to_string())
            .or_insert(computed)
            .clone()
    }

    /// The nearest typed declaration of a member above `class`, for members with no
    /// type of their own.
    pub(crate) fn typed_ancestor(
        &self,
        class: &str,
        kind: MemberKind,
        name: &str,
    ) -> Option<(ClassRef<'_>, &Member)> {
        members::typed_ancestor(self, class, kind, name)
    }

    /// Targets of members of `classes`, each with the name expected at its range.
    pub(crate) fn member_targets(
        &self,
        classes: &[String],
        kind: MemberKind,
        name: &str,
    ) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = Vec::new();
        for class in classes {
            for (class_ref, member) in self.find_member(class, kind, name) {
                let location = Location {
                    path: class_ref.path.to_path_buf(),
                    start: member.name_start,
                    end: member.name_end,
                };
                if !found.iter().any(|(l, _)| *l == location) {
                    found.push((location, member.name.to_ascii_lowercase()));
                }
            }
        }
        found.sort_by(|(a, _), (b, _)| (&a.path, a.start).cmp(&(&b.path, b.start)));
        found
    }

    /// Targets of member `name` on the anonymous class `range` of the file at `path`:
    /// its own declaration, or else the one its parent provides.
    pub(crate) fn anonymous_member_targets(
        &self,
        path: &Path,
        range: &ClassRange,
        kind: MemberKind,
        name: &str,
    ) -> Vec<(Location, String)> {
        match range
            .members
            .iter()
            .find(|m| members::matches(m, kind, name))
        {
            Some(member) => vec![(
                Location {
                    path: path.to_path_buf(),
                    start: member.name_start,
                    end: member.name_end,
                },
                member.name.to_ascii_lowercase(),
            )],
            None => range
                .parent
                .as_ref()
                .map(|parent| self.member_targets(std::slice::from_ref(parent), kind, name))
                .unwrap_or_default(),
        }
    }

    /// Targets of every declaration of the classes in `classes`.
    fn class_targets(&self, classes: &[String]) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = classes
            .iter()
            .flat_map(|class| self.classes_named(class))
            .map(|class_ref| {
                let declaration = class_ref.declaration;
                let location = Location {
                    path: class_ref.path.to_path_buf(),
                    start: declaration.name_start,
                    end: declaration.name_end,
                };
                (location, short_name(&declaration.fqn))
            })
            .collect();
        found.sort_by(|(a, _), (b, _)| (&a.path, a.start).cmp(&(&b.path, b.start)));
        found.dedup_by(|(a, _), (b, _)| a == b);
        found
    }

    /// Definitions of the name at `offset` in `path`, whose current content is
    /// `source`. Results come only from resolved names and are sorted by path and
    /// offset; an unknown or unresolvable name returns nothing.
    ///
    /// A target file that no longer holds the declared name at the indexed range
    /// changed since it was indexed: it is indexed again and the query runs once more.
    ///
    /// On a declaration, the frameworks may answer instead of the declaration itself,
    /// as for a hook implementation and its invocations.
    pub fn definition(&mut self, path: &Path, source: &[u8], offset: u32) -> Vec<Location> {
        self.definition_with(path, source, offset, true)
    }

    /// What the cursor names, for the features built on it (references, hover, call
    /// and type hierarchies, implementations): definitions, with a declaration
    /// standing for itself.
    pub fn targets(&mut self, path: &Path, source: &[u8], offset: u32) -> Vec<Location> {
        self.definition_with(path, source, offset, false)
    }

    fn definition_with(
        &mut self,
        path: &Path,
        source: &[u8],
        offset: u32,
        declarations: bool,
    ) -> Vec<Location> {
        let found = self.resolve(path, source, offset, declarations);
        let mut stale: Vec<PathBuf> = found
            .iter()
            .filter(|(location, name)| !holds_name(location, name))
            .map(|(location, _)| location.path.clone())
            .collect();
        stale.sort();
        stale.dedup();
        if stale.is_empty() {
            return found.into_iter().map(|(location, _)| location).collect();
        }
        self.update(&stale, &[]);
        self.resolve(path, source, offset, declarations)
            .into_iter()
            .map(|(location, _)| location)
            .collect()
    }

    /// Implementations of what the cursor is on. On a class-like, declared or
    /// referenced: its non-abstract subtypes, transitively. On a method: for each
    /// non-abstract subtype of the declaring class, the non-abstract declaration that
    /// subtype uses, when it is not the original. Sorted and deduplicated; stale
    /// targets are re-indexed like definitions.
    pub fn implementations(&mut self, path: &Path, source: &[u8], offset: u32) -> Vec<Location> {
        if is_framework_file(path) {
            return Vec::new();
        }
        // What the cursor is on is read from the index, so the cursor file must be current.
        let key = self.filter.under_root(path);
        if self.ids.contains_key(&key) {
            self.update(&[key], &[]);
        }
        let call = self.call_site(path, source, offset);
        let targets = match call {
            Some(_) => Vec::new(),
            None => self.targets(path, source, offset),
        };
        let compute = |index: &Index| {
            let mut found = match &call {
                Some((classes, name)) => index.call_site_targets(classes, name),
                None => index.implementation_targets(&targets),
            };
            found.retain(|(location, _)| !index.is_stub(&location.path));
            found
        };
        let found = compute(self);
        let mut stale: Vec<PathBuf> = found
            .iter()
            .filter(|(location, name)| !holds_name(location, name))
            .map(|(location, _)| location.path.clone())
            .collect();
        stale.sort();
        stale.dedup();
        if stale.is_empty() {
            return found.into_iter().map(|(location, _)| location).collect();
        }
        self.update(&stale, &[]);
        compute(self)
            .into_iter()
            .map(|(location, _)| location)
            .collect()
    }

    /// The receiver classes and method name when the cursor is on a method call:
    /// `$x->m()`, `$x?->m()` or `X::m()`. The classes are empty for an unknown receiver.
    fn call_site(&self, path: &Path, source: &[u8], offset: u32) -> Option<(Vec<String>, String)> {
        let summary = summarize(source);
        let covers = |start: u32, end: u32| start <= offset && offset <= end;
        if let Some(call) = summary.static_refs.iter().find(|r| covers(r.start, r.end)) {
            if call.kind != MemberKind::Method {
                return None;
            }
            let class = match &call.class {
                ClassSel::Named(fqn) => Some(fqn.clone()),
                ClassSel::SelfKw | ClassSel::StaticKw => {
                    summary.enclosing_class(offset).and_then(|c| c.fqn.clone())
                }
                ClassSel::ParentKw => summary.enclosing_class(offset)?.parent.clone(),
            };
            return Some((class.into_iter().collect(), call.name.clone()));
        }
        match infer::member_at(self, path, &summary, source, offset)? {
            (Receiver::Classes(classes), MemberKind::Method, name) => Some((classes, name)),
            _ => None,
        }
    }

    /// For a call on `classes`: every concrete class the receiver can be (each class
    /// and its subtypes), with the non-abstract method declaration that class uses.
    fn call_site_targets(&self, classes: &[String], name: &str) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = Vec::new();
        for class in classes {
            let mut receivers = self.classes_named(class);
            receivers.extend(self.all_subtypes(class));
            for receiver in receivers {
                if !is_concrete(receiver.declaration) {
                    continue;
                }
                let Some((declaring, member)) =
                    members::member_from(self, receiver, MemberKind::Method, name)
                else {
                    continue;
                };
                if member.is_abstract {
                    continue;
                }
                let location = Location {
                    path: declaring.path.to_path_buf(),
                    start: member.name_start,
                    end: member.name_end,
                };
                if !found.iter().any(|(l, _)| *l == location) {
                    found.push((location, member.name.to_ascii_lowercase()));
                }
            }
        }
        found.sort_by(|(a, _), (b, _)| (&a.path, a.start).cmp(&(&b.path, b.start)));
        found
    }

    fn implementation_targets(&self, targets: &[Location]) -> Vec<(Location, String)> {
        let mut found: Vec<(Location, String)> = Vec::new();
        let mut push = |path: &Path, start: u32, end: u32, name: &str| {
            let location = Location {
                path: path.to_path_buf(),
                start,
                end,
            };
            if !found.iter().any(|(l, _)| *l == location) {
                found.push((location, name.to_ascii_lowercase()));
            }
        };
        for target in targets {
            let Some(file) = self
                .ids
                .get(&target.path)
                .and_then(|&id| self.files[id as usize].as_ref())
            else {
                continue;
            };
            for declaration in &file.declarations {
                let Some(shape) = &declaration.class else {
                    continue;
                };
                if declaration.name_start == target.start {
                    for sub in self.all_subtypes(&declaration.fqn) {
                        if is_concrete(sub.declaration) {
                            let d = sub.declaration;
                            push(sub.path, d.name_start, d.name_end, &short_name(&d.fqn));
                        }
                    }
                    for framework in self.frameworks_for(&target.path) {
                        for (location, name) in
                            framework.class_implementations(self, &declaration.fqn)
                        {
                            push(&location.path, location.start, location.end, &name);
                        }
                    }
                    continue;
                }
                let Some(method) = shape
                    .members
                    .iter()
                    .find(|m| m.kind == MemberKind::Method && m.name_start == target.start)
                else {
                    continue;
                };
                for sub in self.all_subtypes(&declaration.fqn) {
                    if !is_concrete(sub.declaration) {
                        continue;
                    }
                    if let Some((declaring, member)) =
                        members::member_from(self, sub, MemberKind::Method, &method.name)
                    {
                        let original = declaring.path == file.path.as_path()
                            && member.name_start == method.name_start;
                        if !member.is_abstract && !original {
                            push(
                                declaring.path,
                                member.name_start,
                                member.name_end,
                                &member.name,
                            );
                        }
                    }
                }
            }
        }
        found.sort_by(|(a, _), (b, _)| (&a.path, a.start).cmp(&(&b.path, b.start)));
        found
    }

    /// What the active frameworks resolve a string argument at `offset` to.
    fn framework_strings(
        &self,
        path: &Path,
        summary: &FileSummary,
        source: &[u8],
        offset: u32,
    ) -> Option<Vec<(Location, String)>> {
        if self.frameworks.is_empty() {
            return None;
        }
        let argument = infer::string_argument_at(self, path, summary, source, offset)?;
        let call = FrameworkCall {
            targets: &argument.targets,
            strings: &argument.strings,
            receiver: Some(&argument.receiver),
        };
        let found: Vec<(Location, String)> = self
            .frameworks_for(path)
            .flat_map(|f| f.string_targets(self, &call, argument.position, &argument.value))
            .collect();
        (!found.is_empty()).then_some(found)
    }

    /// What the frameworks give for a string literal inside an attribute.
    fn framework_attribute(
        &self,
        path: &Path,
        summary: &FileSummary,
        source: &[u8],
        offset: u32,
    ) -> Option<Vec<(Location, String)>> {
        self.frameworks_for(path).next()?;
        let found = infer::attribute_string_at(summary, source, offset)?;
        let targets: Vec<(Location, String)> = self
            .frameworks_for(path)
            .flat_map(|f| f.attribute_targets(self, &found, offset))
            .collect();
        (!targets.is_empty()).then_some(targets)
    }

    /// What the frameworks give for a cursor on the name of a method call, from what
    /// it targets and the types of its arguments.
    fn framework_call(
        &self,
        path: &Path,
        summary: &FileSummary,
        source: &[u8],
        offset: u32,
    ) -> Option<Vec<(Location, String)>> {
        self.frameworks_for(path).next()?;
        let found = infer::call_at(self, path, summary, source, offset)?;
        let call = FrameworkCall {
            targets: &found.parts.targets,
            strings: &found.parts.strings,
            receiver: Some(&found.parts.receiver),
        };
        let targets: Vec<(Location, String)> = self
            .frameworks_for(path)
            .flat_map(|f| f.call_targets(self, &call, &found.arguments))
            .collect();
        (!targets.is_empty()).then_some(targets)
    }

    /// What the frameworks give for a string literal that is the value of an array key.
    fn framework_keyed_strings(
        &self,
        path: &Path,
        summary: &FileSummary,
        source: &[u8],
        offset: u32,
    ) -> Option<Vec<(Location, String)>> {
        if self.frameworks.is_empty() {
            return None;
        }
        let keyed = infer::keyed_string_at(self, path, summary, source, offset)?;
        let call = keyed.call.as_ref().map(|argument| {
            let call = FrameworkCall {
                targets: &argument.targets,
                strings: &argument.strings,
                receiver: Some(&argument.receiver),
            };
            (call, argument.position)
        });
        let found: Vec<(Location, String)> = self
            .frameworks_for(path)
            .flat_map(|f| {
                f.keyed_string_targets(
                    self,
                    &keyed.key,
                    &keyed.value,
                    call.as_ref().map(|(call, position)| (call, *position)),
                )
            })
            .collect();
        (!found.is_empty()).then_some(found)
    }

    /// What the frameworks give for a cursor on a function or method declaration's
    /// name, or inside an attribute of a function, method or class but not on a
    /// class-like name.
    fn framework_declaration(
        &self,
        path: &Path,
        summary: &FileSummary,
        offset: u32,
    ) -> Option<Vec<(Location, String)>> {
        if self.frameworks.is_empty() {
            return None;
        }
        let covers = |(start, end): (u32, u32)| start <= offset && offset <= end;
        let on_name = name_at(&summary.names, offset).is_some()
            || summary.static_refs.iter().any(|r| covers((r.start, r.end)))
            || summary.keywords.iter().any(|k| covers((k.start, k.end)));
        let in_attributes = |metadata: &[Metadata]| {
            !on_name && metadata.iter().any(|m| !m.annotation && covers(m.span))
        };
        let declaration = summary.declarations.iter().find_map(|d| {
            if d.kind == SymbolKind::Function
                && (covers((d.name_start, d.name_end)) || in_attributes(&d.metadata))
            {
                return Some(FrameworkDeclaration::Function {
                    path,
                    declaration: d,
                    file: &summary.declarations,
                });
            }
            let shape = d.class.as_ref()?;
            if let Some(member) = shape.members.iter().find(|m| {
                m.kind == MemberKind::Method
                    && (covers((m.name_start, m.name_end)) || in_attributes(&m.metadata))
            }) {
                return Some(FrameworkDeclaration::Method {
                    path,
                    class: d,
                    member,
                });
            }
            let metadata = shape
                .metadata
                .iter()
                .find(|m| !on_name && !m.annotation && covers(m.span))?;
            Some(FrameworkDeclaration::Class {
                path,
                class: d,
                metadata,
            })
        })?;
        let found: Vec<(Location, String)> = self
            .frameworks_for(path)
            .flat_map(|f| f.declaration_targets(self, &declaration))
            .collect();
        (!found.is_empty()).then_some(found)
    }

    /// Each target with the lowercased short name the index expects at its range.
    /// With `declarations`, the frameworks answer for the declaration under the cursor,
    /// and for YAML files.
    fn resolve(
        &self,
        path: &Path,
        source: &[u8],
        offset: u32,
        declarations: bool,
    ) -> Vec<(Location, String)> {
        if is_framework_file(path) {
            if !declarations {
                return Vec::new();
            }
            return self
                .frameworks_for(path)
                .flat_map(|f| f.file_targets(self, path, source, offset))
                .collect();
        }
        let summary = summarize(source);

        if declarations && let Some(found) = self.framework_declaration(path, &summary, offset) {
            return found;
        }

        if let Some(declaration) = summary
            .declarations
            .iter()
            .find(|d| d.name_start <= offset && offset <= d.name_end)
        {
            let location = Location {
                path: path.to_path_buf(),
                start: declaration.name_start,
                end: declaration.name_end,
            };
            return vec![(location, short_name(&declaration.fqn))];
        }

        let declared_member = summary
            .declarations
            .iter()
            .filter_map(|d| d.class.as_ref())
            .flat_map(|shape| shape.members.iter())
            .find(|m| m.name_start <= offset && offset <= m.name_end);
        if let Some(member) = declared_member {
            let location = Location {
                path: path.to_path_buf(),
                start: member.name_start,
                end: member.name_end,
            };
            return vec![(location, member.name.to_ascii_lowercase())];
        }

        let covers = |start: u32, end: u32| start <= offset && offset <= end;
        let class_of = |sel: &ClassSel| -> Option<String> {
            match sel {
                ClassSel::Named(fqn) => Some(fqn.clone()),
                ClassSel::SelfKw | ClassSel::StaticKw => {
                    summary.enclosing_class(offset).and_then(|c| c.fqn.clone())
                }
                ClassSel::ParentKw => summary.enclosing_class(offset)?.parent.clone(),
            }
        };
        if let Some(keyword) = summary.keywords.iter().find(|k| covers(k.start, k.end)) {
            return class_of(&keyword.class)
                .map(|class| self.class_targets(&[class]))
                .unwrap_or_default();
        }
        if let Some(member) = summary.static_refs.iter().find(|r| covers(r.start, r.end)) {
            let anonymous = summary.enclosing_class(offset).filter(|r| r.fqn.is_none());
            if let (ClassSel::SelfKw | ClassSel::StaticKw, Some(range)) = (&member.class, anonymous)
            {
                return self.anonymous_member_targets(path, range, member.kind, &member.name);
            }
            return class_of(&member.class)
                .map(|class| self.member_targets(&[class], member.kind, &member.name))
                .unwrap_or_default();
        }
        if let Some(member) = summary.doc_members.iter().find(|r| covers(r.start, r.end)) {
            let Some(class) = class_of(&member.class) else {
                return Vec::new();
            };
            return match member.kind {
                Some(kind) => self.member_targets(&[class], kind, &member.name),
                // A constant of that exact name, else a method.
                None => {
                    let constants = self.member_targets(
                        std::slice::from_ref(&class),
                        MemberKind::Constant,
                        &member.name,
                    );
                    if constants.is_empty() {
                        self.member_targets(&[class], MemberKind::Method, &member.name)
                    } else {
                        constants
                    }
                }
            };
        }
        if declarations
            && let Some(found) = self.framework_attribute(path, &summary, source, offset)
        {
            return found;
        }
        // Only a definition request: hover, references and hierarchies keep the
        // method's own declaration.
        if declarations && let Some(found) = self.framework_call(path, &summary, source, offset) {
            return found;
        }
        if let Some((receiver, kind, name)) = infer::member_at(self, path, &summary, source, offset)
        {
            return match receiver {
                Receiver::Classes(classes) => self.member_targets(&classes, kind, &name),
                Receiver::Anonymous(i) => {
                    self.anonymous_member_targets(path, &summary.class_ranges[i], kind, &name)
                }
            };
        }

        if let Some(found) = self.framework_strings(path, &summary, source, offset) {
            return found;
        }
        if let Some(found) = self.framework_keyed_strings(path, &summary, source, offset) {
            return found;
        }

        let Some(name) =
            name_at(&summary.names, offset).or_else(|| name_at(&summary.doc_names, offset))
        else {
            return name_at(&summary.annotation_names, offset)
                .map(|name| self.class_targets(&self.annotation_classes(&summary, name)))
                .unwrap_or_default();
        };
        let (key, holder, entries): (String, &Index, &[Entry]) = match name.kind {
            NameKind::ClassLike => {
                let key = name.fqn.to_ascii_lowercase();
                let (holder, entries) = self.entries(SymbolKind::Class, &key);
                (key, holder, entries)
            }
            NameKind::Function => {
                let key = name.fqn.to_ascii_lowercase();
                let (holder, found) = self.entries(SymbolKind::Function, &key);
                match key.rsplit_once('\\') {
                    // PHP falls back to the global function only for unqualified,
                    // non-imported names.
                    Some((_, global)) if found.is_empty() && !name.qualified && !name.imported => {
                        let global = global.to_string();
                        let (holder, entries) = self.entries(SymbolKind::Function, &global);
                        (global, holder, entries)
                    }
                    _ => (key, holder, found),
                }
            }
            NameKind::Constant => {
                let key = symbol_key(SymbolKind::Constant, &name.fqn);
                let (holder, found) = self.entries(SymbolKind::Constant, &key);
                match name.fqn.rsplit_once('\\') {
                    // Unqualified constants fall back to the global one, like functions.
                    Some((_, global)) if found.is_empty() && !name.qualified && !name.imported => {
                        let global = global.to_string();
                        let (holder, entries) = self.entries(SymbolKind::Constant, &global);
                        (global, holder, entries)
                    }
                    _ => (key, holder, found),
                }
            }
        };

        let expected = short_name(&key);
        let mut found: Vec<(Location, String)> = entries
            .iter()
            .map(|entry| {
                let file = holder.files[entry.file as usize]
                    .as_ref()
                    .expect("symbol tables only reference live files");
                let location = Location {
                    path: file.path.clone(),
                    start: entry.start,
                    end: entry.end,
                };
                (location, expected.clone())
            })
            .collect();
        found.sort_by(|(a, _), (b, _)| (&a.path, a.start).cmp(&(&b.path, b.start)));
        found
    }

    /// Removes the indexed files in `files`, and those equal to or inside any of
    /// `deleted`, which may name directories. Returns how many were removed.
    ///
    /// Deleted paths that are not indexed files are checked in one pass over the
    /// index, looking each file's ancestors up in a set, so a batch costs the same
    /// however many unrelated paths it holds.
    fn remove_paths(&mut self, files: &[PathBuf], deleted: &[PathBuf]) -> usize {
        let mut ids: Vec<u32> = files
            .iter()
            .filter_map(|p| self.ids.get(p))
            .copied()
            .collect();
        let mut directories: HashSet<&Path> = HashSet::new();
        for path in deleted {
            match self.ids.get(path) {
                Some(&id) => ids.push(id),
                None => {
                    directories.insert(path);
                }
            }
        }
        if !directories.is_empty() {
            ids.extend(
                self.ids
                    .iter()
                    .filter(|(path, _)| {
                        path.ancestors()
                            .skip(1)
                            .any(|ancestor| directories.contains(ancestor))
                    })
                    .map(|(_, &id)| id),
            );
        }
        ids.sort_unstable();
        ids.dedup();
        for &id in &ids {
            self.remove(id);
        }
        ids.len()
    }

    fn insert(&mut self, file: CachedFile) {
        let id = self.files.len() as u32;
        for (decl, declaration) in file.declarations.iter().enumerate() {
            self.table_mut(declaration.kind)
                .entry(symbol_key(declaration.kind, &declaration.fqn))
                .or_default()
                .push(Entry {
                    file: id,
                    decl: decl as u32,
                    start: declaration.name_start,
                    end: declaration.name_end,
                });
        }
        for (decl, declaration) in file.declarations.iter().enumerate() {
            for supertype in supertype_names(declaration) {
                self.subtype_table
                    .entry(supertype.to_ascii_lowercase())
                    .or_default()
                    .push(Entry {
                        file: id,
                        decl: decl as u32,
                        start: declaration.name_start,
                        end: declaration.name_end,
                    });
            }
        }
        for key in &file.ref_keys {
            self.candidates.entry(*key).or_default().push(id);
        }
        for name in declared_methods(&file) {
            *self.method_names.entry(name).or_default() += 1;
        }
        for namespace in declared_namespaces(&file) {
            *self.namespaces.entry(namespace).or_default() += 1;
        }
        self.ids.insert(file.path.clone(), id);
        self.files.push(Some(file));
    }

    fn remove(&mut self, id: u32) {
        let Some(file) = self.files[id as usize].take() else {
            return;
        };
        self.ids.remove(&file.path);
        for name in declared_methods(&file) {
            if let Some(count) = self.method_names.get_mut(&name) {
                *count -= 1;
                if *count == 0 {
                    self.method_names.remove(&name);
                }
            }
        }
        for namespace in declared_namespaces(&file) {
            if let Some(count) = self.namespaces.get_mut(&namespace) {
                *count -= 1;
                if *count == 0 {
                    self.namespaces.remove(&namespace);
                }
            }
        }
        for key in &file.ref_keys {
            if let Some(files) = self.candidates.get_mut(key) {
                files.retain(|f| *f != id);
                if files.is_empty() {
                    self.candidates.remove(key);
                }
            }
        }
        for declaration in &file.declarations {
            for supertype in supertype_names(declaration) {
                let key = supertype.to_ascii_lowercase();
                if let Some(entries) = self.subtype_table.get_mut(&key) {
                    entries.retain(|e| e.file != id);
                    if entries.is_empty() {
                        self.subtype_table.remove(&key);
                    }
                }
            }
            let key = symbol_key(declaration.kind, &declaration.fqn);
            let table = self.table_mut(declaration.kind);
            if let Some(entries) = table.get_mut(&key) {
                entries.retain(|e| e.file != id);
                if entries.is_empty() {
                    table.remove(&key);
                }
            }
        }
    }

    fn table_mut(&mut self, kind: SymbolKind) -> &mut HashMap<String, Vec<Entry>> {
        match kind {
            SymbolKind::Function => &mut self.functions,
            SymbolKind::Constant => &mut self.constants,
            _ => &mut self.classes,
        }
    }
}

/// A class or enum that is not abstract.
fn is_concrete(declaration: &Declaration) -> bool {
    matches!(declaration.kind, SymbolKind::Class | SymbolKind::Enum)
        && declaration
            .class
            .as_ref()
            .is_some_and(|shape| !shape.is_abstract)
}

/// The parent and interfaces a class-like declaration names.
fn supertype_names(declaration: &Declaration) -> impl Iterator<Item = &str> {
    declaration.class.iter().flat_map(|shape| {
        shape
            .parent
            .iter()
            .chain(shape.interfaces.iter())
            .map(String::as_str)
    })
}

/// Every lowercased namespace a file's declarations live in, with its parents.
fn declared_namespaces(file: &CachedFile) -> impl Iterator<Item = String> + '_ {
    file.declarations.iter().flat_map(|declaration| {
        let lower = declaration.fqn.to_ascii_lowercase();
        let namespace = lower.rsplit_once('\\').map_or("", |(ns, _)| ns).to_string();
        let mut prefixes = Vec::new();
        let mut end = 0;
        while let Some(found) = namespace[end..].find('\\') {
            prefixes.push(namespace[..end + found].to_string());
            end += found + 1;
        }
        if !namespace.is_empty() {
            prefixes.push(namespace);
        }
        prefixes
    })
}

/// The lowercased method names a file declares, trait aliases included.
fn declared_methods(file: &CachedFile) -> impl Iterator<Item = String> + '_ {
    file.declarations
        .iter()
        .filter_map(|d| d.class.as_ref())
        .flat_map(|shape| {
            shape
                .members
                .iter()
                .filter(|m| m.kind == MemberKind::Method)
                .map(|m| m.name.to_ascii_lowercase())
                .chain(shape.aliases.iter().cloned())
        })
}

fn lookup<'a>(table: &'a HashMap<String, Vec<Entry>>, key: &str) -> &'a [Entry] {
    table.get(key).map(Vec::as_slice).unwrap_or_default()
}

/// The start offsets of every member access in `source`: `->`, `?->` and `::`
/// accesses whose member is named in the code.
pub fn member_access_offsets(source: &[u8]) -> Vec<u32> {
    let mut offsets = infer::member_access_starts(source);
    offsets.extend(summarize(source).static_refs.iter().map(|r| r.start));
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

/// The table key of a symbol: lowercased for classes and functions, which PHP
/// matches case-insensitively; for constants only the namespace part is lowercased.
fn symbol_key(kind: SymbolKind, fqn: &str) -> String {
    match (kind, fqn.rsplit_once('\\')) {
        (SymbolKind::Constant, Some((namespace, name))) => {
            format!("{}\\{name}", namespace.to_ascii_lowercase())
        }
        (SymbolKind::Constant, None) => fqn.to_string(),
        _ => fqn.to_ascii_lowercase(),
    }
}

fn short_name(fqn: &str) -> String {
    fqn.rsplit('\\').next().unwrap_or(fqn).to_ascii_lowercase()
}

/// Whether the file at `location` still holds `name` (lowercased) at its range.
fn holds_name(location: &Location, name: &str) -> bool {
    std::fs::read(&location.path).is_ok_and(|text| {
        text.get(location.start as usize..location.end as usize)
            .is_some_and(|found| found.eq_ignore_ascii_case(name.as_bytes()))
    })
}

/// The name covering `offset`. A name that strictly contains the offset wins over
/// one that ends exactly at it.
fn name_at(names: &[NameRef], offset: u32) -> Option<&NameRef> {
    names
        .iter()
        .find(|n| n.start <= offset && offset < n.end)
        .or_else(|| names.iter().find(|n| n.end == offset))
}

/// A declaration without the text only hover shows: signatures and docblocks are
/// read again from the file when needed, which keeps the index and its cache small.
fn lean(mut declaration: Declaration) -> Declaration {
    declaration.signature = String::new();
    declaration.doc = None;
    if let Some(class) = &mut declaration.class {
        for member in &mut class.members {
            member.signature = String::new();
            member.doc = None;
        }
    }
    declaration
}

/// Size and modification time in nanoseconds.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), mtime.as_nanos() as u64))
}

/// Reads and summarizes files in parallel. The stamp is taken before reading, so a
/// file written meanwhile looks changed to the next cache check. Unreadable files
/// yield no declarations rather than failing.
fn read_files(paths: &[PathBuf]) -> Vec<CachedFile> {
    read_files_with(paths, summarize)
}

/// `read_files` with `summarize_with` in place of `summarize`.
fn read_files_with(paths: &[PathBuf], summarize_with: fn(&[u8]) -> FileSummary) -> Vec<CachedFile> {
    paths
        .par_iter()
        .map(|path| {
            let (size, mtime_ns) = stamp(path).unwrap_or_default();
            let (declarations, has_errors, ref_keys) = match std::fs::read(path) {
                Ok(source) => {
                    let summary = summarize_with(&source);
                    let declarations = summary.declarations.into_iter().map(lean).collect();
                    (declarations, summary.has_parse_errors, summary.ref_keys)
                }
                Err(_) => (Vec::<Declaration>::new(), false, Vec::new()),
            };
            CachedFile {
                path: path.clone(),
                size,
                mtime_ns,
                has_errors,
                declarations,
                ref_keys,
            }
        })
        .collect()
}
