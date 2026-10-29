//! Declarations, class shapes and resolved names of one PHP file.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hasher};

use mago_allocator::LocalArena;
use mago_database::file::FileId;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::comments::docblock::get_docblock_before_position;
use mago_syntax::cst::{
    Access, AnonymousClass, Argument, ArrayElement, ArrowFunction, Attribute, AttributeList, Class,
    ClassConstantAccess, ClassLikeConstantSelector, ClassLikeMember, ClassLikeMemberSelector,
    Closure, Constant, ConstantAccess, Enum, EnumCaseItem, Expression, Function, FunctionCall,
    FunctionLikeParameterList, FunctionLikeReturnTypeHint, FunctionPartialApplication, Hint,
    Instantiation, Interface, Literal, Method, MethodCall, Modifier, Namespace, NullSafeMethodCall,
    NullSafePropertyAccess, PartialArgument, Program, Property, PropertyAccess, PropertyItem,
    Sequence, StaticMethodCall, StaticMethodPartialApplication, StaticPropertyAccess, Trait,
    TraitUseAdaptation, TraitUseSpecification, TriviaKind, Use, UseItems, UseType, Variable,
};
use mago_syntax::parser::parse_file_content;
use mago_syntax::walker::Walker;
use serde::{Deserialize, Serialize};

use crate::metadata::{self, MetaValue, Metadata};
use crate::phpdoc::{
    NameContext, SeeKind, SeeName, TypeExpr, doc_class_names, doc_see_names, parse_docblock,
    parse_type,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Class,
    Interface,
    Trait,
    Enum,
    Function,
    Constant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    ClassLike,
    Function,
    Constant,
}

/// A symbol whose references can be searched. Constants keep their name's case;
/// everything else is matched case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RefKey {
    Class(String),
    Function(String),
    Constant(String),
    Member(String),
    /// An unqualified annotation name no import resolves, by its short name: the
    /// frameworks may resolve it to any class with that short name.
    Annotation(String),
}

impl RefKey {
    /// A stable hash of the normalized key. Two keys may collide; that only adds a
    /// candidate file that verification drops.
    pub(crate) fn hash(&self) -> u64 {
        let (prefix, body) = match self {
            RefKey::Class(fqn) => (b'c', bare(fqn).to_ascii_lowercase()),
            RefKey::Function(fqn) => (b'f', bare(fqn).to_ascii_lowercase()),
            RefKey::Constant(fqn) => (
                b'k',
                match bare(fqn).rsplit_once('\\') {
                    Some((namespace, name)) => {
                        format!("{}\\{name}", namespace.to_ascii_lowercase())
                    }
                    None => bare(fqn).to_string(),
                },
            ),
            RefKey::Member(name) => (b'm', name.to_ascii_lowercase()),
            RefKey::Annotation(short) => (b'a', short.to_ascii_lowercase()),
        };
        let mut hasher = DefaultHasher::new();
        hasher.write_u8(prefix);
        hasher.write(body.as_bytes());
        hasher.finish()
    }
}

fn bare(fqn: &str) -> &str {
    fqn.trim_start_matches('\\')
}

/// A declared symbol. `fqn` keeps the source casing and has no leading backslash;
/// `name_start..name_end` covers the declared name token only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Declaration {
    pub kind: SymbolKind,
    pub fqn: String,
    pub name_start: u32,
    pub name_end: u32,
    /// The declaration head on one line, without the body.
    #[serde(default)]
    pub signature: String,
    /// The raw docblock text.
    #[serde(default)]
    pub doc: Option<String>,
    /// Functions: the return type, PHPDoc first when it names a class.
    #[serde(default)]
    pub returns: Option<TypeExpr>,
    /// Class-likes: parent, interfaces, traits and members.
    #[serde(default)]
    pub class: Option<ClassShape>,
    /// The whole declaration, body included.
    #[serde(default)]
    pub range: (u32, u32),
    /// Functions: their attributes. Class-likes keep theirs in `ClassShape`.
    #[serde(default)]
    pub metadata: Vec<Metadata>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClassShape {
    pub parent: Option<String>,
    /// `implements` for classes and enums, `extends` for interfaces.
    pub interfaces: Vec<String>,
    pub traits: Vec<String>,
    pub members: Vec<Member>,
    /// An abstract class or an interface.
    #[serde(default)]
    pub is_abstract: bool,
    /// Declares `__call` or `__callStatic`, or its docblock has an `@method` or
    /// `@mixin` tag: it may answer methods it does not declare.
    #[serde(default)]
    pub magic: bool,
    /// Lowercased method names that trait `as` adaptations add.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The class-like's attributes, then its docblock annotations.
    #[serde(default)]
    pub metadata: Vec<Metadata>,
    /// `@template` names, as the class's member types write them.
    #[serde(default)]
    pub templates: Vec<String>,
    /// The arguments the class-like gives generic parents and interfaces through
    /// `@extends` and `@implements`.
    #[serde(default)]
    pub generic_supertypes: Vec<(String, Vec<TypeExpr>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemberKind {
    Method,
    Property,
    Constant,
    EnumCase,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    pub kind: MemberKind,
    /// Properties without `$`.
    pub name: String,
    pub is_static: bool,
    pub name_start: u32,
    pub name_end: u32,
    pub signature: String,
    pub doc: Option<String>,
    /// Property type or method return type, PHPDoc first when it names a class.
    pub ty: Option<TypeExpr>,
    /// Methods: parameter names without `$`, with their types.
    pub params: Vec<(String, Option<TypeExpr>)>,
    /// Methods: each parameter's declared type, PHPDoc left out.
    #[serde(default)]
    pub hints: Vec<Option<TypeExpr>>,
    /// An abstract method or an interface method.
    #[serde(default)]
    pub is_abstract: bool,
    /// Declared `private`: not inherited by subclasses.
    #[serde(default)]
    pub is_private: bool,
    /// The whole member, body included.
    #[serde(default)]
    pub range: (u32, u32),
    /// Methods: their attributes.
    #[serde(default)]
    pub metadata: Vec<Metadata>,
    /// Methods: their own `@template` names, which each call binds from its
    /// arguments.
    #[serde(default)]
    pub templates: Vec<String>,
}

/// A use of a name, resolved against the file's namespace and imports.
/// `qualified` is true when the source text of the name contains a backslash;
/// `imported` is true when a `use` statement resolved it.
#[derive(Clone, Debug)]
pub struct NameRef {
    pub kind: NameKind,
    pub fqn: String,
    pub start: u32,
    pub end: u32,
    pub qualified: bool,
    pub imported: bool,
}

/// The namespaces and class imports of a file, for resolving names written in
/// docblocks and type hints.
#[derive(Clone, Debug)]
pub struct FileNames {
    scopes: Vec<NameScope>,
}

#[derive(Clone, Debug, Default)]
struct NameScope {
    start: u32,
    end: u32,
    namespace: String,
    /// Lowercased alias to fully qualified name.
    imports: HashMap<String, String>,
    /// `use function` imports: lowercased alias to fully qualified name.
    function_imports: HashMap<String, String>,
}

impl Default for FileNames {
    fn default() -> Self {
        FileNames {
            scopes: vec![NameScope {
                end: u32::MAX,
                ..NameScope::default()
            }],
        }
    }
}

impl FileNames {
    /// The names in effect at `offset`.
    pub fn at(&self, offset: u32) -> ScopeNames<'_> {
        ScopeNames {
            scope: self.scope_index(offset).map(|i| &self.scopes[i]),
        }
    }

    fn scope_index(&self, offset: u32) -> Option<usize> {
        self.scopes
            .iter()
            .rposition(|s| s.start <= offset && offset <= s.end)
    }
}

pub struct ScopeNames<'a> {
    scope: Option<&'a NameScope>,
}

impl ScopeNames<'_> {
    pub fn namespace(&self) -> &str {
        self.scope.map_or("", |s| s.namespace.as_str())
    }

    /// The function a `use function` import gives the unqualified name `name`.
    pub fn imported_function(&self, name: &str) -> Option<&str> {
        self.scope?
            .function_imports
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

impl NameContext for ScopeNames<'_> {
    fn is_imported(&self, name: &str) -> bool {
        let first = name.split('\\').next().unwrap_or(name);
        self.scope
            .is_some_and(|s| s.imports.contains_key(&first.to_ascii_lowercase()))
    }

    fn resolve_class(&self, name: &str) -> String {
        let Some(scope) = self.scope else {
            return name.to_string();
        };
        let (first, rest) = match name.split_once('\\') {
            Some((first, rest)) => (first, Some(rest)),
            None => (name, None),
        };
        if let Some(target) = scope.imports.get(&first.to_ascii_lowercase()) {
            return match rest {
                Some(rest) => format!("{target}\\{rest}"),
                None => target.clone(),
            };
        }
        if scope.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}\\{name}", scope.namespace)
        }
    }
}

/// The class a static access or class keyword refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassSel {
    Named(String),
    SelfKw,
    StaticKw,
    ParentKw,
}

/// `X::method()`, `X::CONST`, `X::Case` or `X::$prop`. `kind` is `Constant` for
/// both constants and enum cases; `name` has no `$`; the range covers the member
/// name as written, `$` included.
#[derive(Clone, Debug)]
pub struct StaticRef {
    pub kind: MemberKind,
    pub name: String,
    pub start: u32,
    pub end: u32,
    pub class: ClassSel,
}

/// An occurrence of `self`, `static` or `parent` naming a class.
#[derive(Clone, Debug)]
pub struct KeywordRef {
    pub start: u32,
    pub end: u32,
    pub class: ClassSel,
}

/// The source range of a class-like declaration, for finding the enclosing class.
/// Anonymous classes have no `fqn` and carry their own `members`, since the index
/// cannot name them; named classes leave `members` empty.
#[derive(Clone, Debug)]
pub struct ClassRange {
    pub start: u32,
    pub end: u32,
    pub fqn: Option<String>,
    pub parent: Option<String>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Default)]
pub struct FileSummary {
    pub declarations: Vec<Declaration>,
    pub names: Vec<NameRef>,
    /// Names written in docblocks, sorted by start: classes in type positions and
    /// `@see`-like values, and functions in `@see`-like values. Kept apart from
    /// `names`, which diagnostics and call hierarchy read.
    pub doc_names: Vec<NameRef>,
    /// Members named in `@see`-like docblock values, sorted by start. Kept apart
    /// from `static_refs`.
    pub doc_members: Vec<DocMemberRef>,
    /// Annotation names written in docblocks, nested ones included, resolved by
    /// the file's imports; sorted by start.
    pub annotation_names: Vec<NameRef>,
    pub has_parse_errors: bool,
    pub file_names: FileNames,
    pub static_refs: Vec<StaticRef>,
    pub keywords: Vec<KeywordRef>,
    pub class_ranges: Vec<ClassRange>,
    /// Hashes of every `RefKey` the file mentions or declares, sorted and unique.
    pub ref_keys: Vec<u64>,
    /// Lowercased names the file checks with `class_exists`, `interface_exists`,
    /// `trait_exists`, `enum_exists`, `function_exists`, `method_exists` or
    /// `defined`: classes fully qualified, functions and constants as written
    /// without a leading backslash, methods by their bare name. Sorted and unique.
    pub guarded: Vec<String>,
    /// Spans of import statements and attributes, whose names are not uses.
    pub unchecked: Vec<(u32, u32)>,
    /// Spans of type declarations: parameter, return, property and catch types.
    pub hints: Vec<(u32, u32)>,
    /// Spans of closures and arrow functions, whose `$this` and scope can be rebound.
    pub closures: Vec<(u32, u32)>,
}

impl FileSummary {
    /// The innermost class-like declaration containing `offset`.
    pub fn enclosing_class(&self, offset: u32) -> Option<&ClassRange> {
        self.class_ranges
            .iter()
            .filter(|r| r.start <= offset && offset <= r.end)
            .max_by_key(|r| r.start)
    }

    /// The named class a `self`, `static` or `parent` keyword stands for, from the
    /// class around it.
    pub fn keyword_class(&self, keyword: &KeywordRef) -> Option<String> {
        let range = self.enclosing_class(keyword.start)?;
        match keyword.class {
            ClassSel::ParentKw => range.parent.clone(),
            _ => range.fqn.clone(),
        }
    }
}

/// What the docblocks of a file name.
struct DocRefs {
    names: Vec<NameRef>,
    members: Vec<DocMemberRef>,
    annotations: Vec<NameRef>,
}

/// The names every docblock of `program` writes, resolved in the docblock's scope:
/// classes and functions, members, and annotation names. A docblock that is not
/// valid UTF-8 is skipped, so ranges stay exact.
fn doc_names(program: &Program<'_>, names: &FileNames) -> DocRefs {
    let mut found: Vec<NameRef> = Vec::new();
    let mut members: Vec<DocMemberRef> = Vec::new();
    let mut annotations: Vec<NameRef> = Vec::new();
    let docblocks = program
        .trivia
        .iter()
        .filter(|t| t.kind == TriviaKind::DocBlockComment)
        .filter_map(|t| Some((t.span.start.offset, std::str::from_utf8(t.value).ok()?)));
    for (start, text) in docblocks {
        let scope = names.at(start);
        let at = |offset: usize| start + offset as u32;
        for (s, e, fqn) in doc_class_names(text, &scope) {
            found.push(NameRef {
                kind: NameKind::ClassLike,
                fqn,
                start: at(s),
                end: at(e),
                qualified: text[s..e].contains('\\'),
                imported: false,
            });
        }
        let mut spans = Vec::new();
        for annotation in metadata::annotations(text, start, &scope) {
            annotation_spans(&annotation, &mut spans);
        }
        for (s, e, resolved) in spans {
            let written = &text[(s - start) as usize..(e - start) as usize];
            annotations.push(NameRef {
                kind: NameKind::ClassLike,
                fqn: written.strip_prefix('\\').map_or(resolved, str::to_string),
                start: s,
                end: e,
                qualified: written.contains('\\'),
                imported: scope.is_imported(written),
            });
        }
        for see in doc_see_names(text) {
            match see {
                SeeName::Member {
                    class,
                    name,
                    start: s,
                    end: e,
                    kind,
                } => members.push(DocMemberRef {
                    class: match class.to_ascii_lowercase().as_str() {
                        "self" => ClassSel::SelfKw,
                        "static" => ClassSel::StaticKw,
                        "parent" => ClassSel::ParentKw,
                        _ => ClassSel::Named(match class.strip_prefix('\\') {
                            Some(qualified) => qualified.to_string(),
                            None => scope.resolve_class(class),
                        }),
                    },
                    name: name.to_string(),
                    kind: match kind {
                        SeeKind::Method => Some(MemberKind::Method),
                        SeeKind::Property => Some(MemberKind::Property),
                        SeeKind::MethodOrConstant => None,
                    },
                    start: at(s),
                    end: at(e),
                }),
                SeeName::Function {
                    name,
                    start: s,
                    end: e,
                } => {
                    let imported = scope.imported_function(name);
                    let fqn = match (name.strip_prefix('\\'), imported) {
                        (Some(qualified), _) => qualified.to_string(),
                        (None, _) if name.contains('\\') => scope.resolve_class(name),
                        (None, Some(imported)) => imported.to_string(),
                        (None, None) if scope.namespace().is_empty() => name.to_string(),
                        (None, None) => format!("{}\\{name}", scope.namespace()),
                    };
                    found.push(NameRef {
                        kind: NameKind::Function,
                        fqn,
                        start: at(s),
                        end: at(e),
                        qualified: name.contains('\\'),
                        imported: imported.is_some(),
                    });
                }
            }
        }
    }
    found.sort_by_key(|n| n.start);
    members.sort_by_key(|m| m.start);
    annotations.sort_by_key(|n| n.start);
    DocRefs {
        names: found,
        members,
        annotations,
    }
}

/// The name range and resolved name of an annotation and of every annotation
/// nested in its arguments.
fn annotation_spans(annotation: &Metadata, out: &mut Vec<(u32, u32, String)>) {
    out.push((annotation.start, annotation.end, annotation.name.clone()));
    let mut values: Vec<&MetaValue> = annotation.arguments.iter().map(|(_, v)| v).collect();
    while let Some(value) = values.pop() {
        match value {
            MetaValue::Nested(nested) => annotation_spans(nested, out),
            MetaValue::List(entries) => values.extend(entries.iter().map(|(_, v)| v)),
            _ => {}
        }
    }
}

/// A member named in a `@see`-like docblock value: `Foo::bar()`, `Foo::$bar`,
/// `Foo::BAR`. The range covers the member name as written, `$` included.
#[derive(Clone, Debug)]
pub struct DocMemberRef {
    pub class: ClassSel,
    pub name: String,
    /// `None` for `Foo::NAME` without parentheses: a method or a constant.
    pub kind: Option<MemberKind>,
    pub start: u32,
    pub end: u32,
}

/// Parses `source` and returns its declarations and resolved names. Parsing is
/// tolerant: a syntax error does not hide the declarations around it.
pub fn summarize(source: &[u8]) -> FileSummary {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::zero(), source);
    let resolved = NameResolver::new(&arena).resolve(program);

    let mut collector = Collector {
        source,
        program,
        names: FileNames::default(),
        pending: Vec::new(),
        functions: HashSet::new(),
        constants: HashSet::new(),
        not_refs: HashSet::new(),
        static_refs: Vec::new(),
        keywords: Vec::new(),
        anonymous: Vec::new(),
        member_names: HashSet::new(),
        guard_names: Vec::new(),
        guard_classes: Vec::new(),
        unchecked: Vec::new(),
        hints: Vec::new(),
        closures: Vec::new(),
    };
    SyntaxVisitor.walk_program(program, &mut collector);

    let resolved: HashMap<u32, (u32, &[u8], bool)> = resolved
        .iter()
        .map(|(start, end, fqn, imported)| (start, (end, fqn, imported)))
        .collect();
    let fqn_at = |start: u32| resolved.get(&start).map(|(_, fqn, _)| to_fqn(fqn));

    let declaration_starts: HashSet<u32> = collector.pending.iter().map(|p| p.start).collect();
    let resolve_sel = |sel: PendingSel| match sel {
        PendingSel::Named(start) => fqn_at(start).map(ClassSel::Named),
        PendingSel::Keyword(sel) => Some(sel),
    };
    let static_refs: Vec<StaticRef> = collector
        .static_refs
        .into_iter()
        .filter_map(|(kind, name, start, end, sel)| {
            Some(StaticRef {
                kind,
                name,
                start,
                end,
                class: resolve_sel(sel)?,
            })
        })
        .collect();
    let mut class_ranges = Vec::new();
    let declarations: Vec<Declaration> = collector
        .pending
        .into_iter()
        .filter_map(|pending| {
            let fqn = pending.fqn.or_else(|| fqn_at(pending.start))?;
            if let (Some((start, end)), Some(shape)) = (pending.range, &pending.shape) {
                class_ranges.push(ClassRange {
                    start,
                    end,
                    fqn: Some(fqn.clone()),
                    parent: shape.parent.and_then(fqn_at),
                    members: Vec::new(),
                });
            }
            let documented_magic = pending.doc.as_deref().is_some_and(has_magic_tags);
            let class = pending.shape.map(|shape| {
                let mut members = shape.members;
                for (start, attributes) in shape.member_attributes {
                    if let Some(member) = members
                        .iter_mut()
                        .find(|m| m.kind == MemberKind::Method && m.name_start == start)
                    {
                        member.metadata = attributes
                            .into_iter()
                            .filter_map(|meta| meta.resolve(&fqn_at))
                            .collect();
                    }
                }
                let mut interfaces: Vec<String> =
                    shape.interfaces.into_iter().filter_map(fqn_at).collect();
                // PHP adds an enum's interfaces itself; a stub may also name them.
                for implicit in shape.implicit {
                    if !interfaces.iter().any(|i| i.eq_ignore_ascii_case(implicit)) {
                        interfaces.push(implicit.to_string());
                    }
                }
                ClassShape {
                    parent: shape.parent.and_then(fqn_at),
                    interfaces,
                    traits: shape.traits.into_iter().filter_map(fqn_at).collect(),
                    magic: documented_magic || members.iter().any(handles_calls),
                    aliases: shape.aliases,
                    members,
                    is_abstract: shape.is_abstract,
                    metadata: shape
                        .attributes
                        .into_iter()
                        .filter_map(|meta| meta.resolve(&fqn_at))
                        .chain(shape.annotations)
                        .collect(),
                    templates: shape.templates,
                    generic_supertypes: shape.generic_supertypes,
                }
            });
            Some(Declaration {
                kind: pending.kind,
                fqn,
                name_start: pending.start,
                name_end: pending.end,
                signature: pending.signature,
                doc: pending.doc,
                returns: pending.returns,
                class,
                range: pending.span,
                metadata: pending
                    .attributes
                    .into_iter()
                    .filter_map(|meta| meta.resolve(&fqn_at))
                    .collect(),
            })
        })
        .collect();

    class_ranges.extend(collector.anonymous.into_iter().map(|anonymous| ClassRange {
        start: anonymous.start,
        end: anonymous.end,
        fqn: None,
        parent: anonymous.parent.and_then(fqn_at),
        members: anonymous.members,
    }));

    let names: Vec<NameRef> = resolved
        .iter()
        .filter(|(start, _)| {
            !collector.not_refs.contains(start) && !declaration_starts.contains(start)
        })
        .map(|(&start, &(end, fqn, imported))| {
            let kind = if collector.functions.contains(&start) {
                NameKind::Function
            } else if collector.constants.contains(&start) {
                NameKind::Constant
            } else {
                NameKind::ClassLike
            };
            let text = source.get(start as usize..end as usize).unwrap_or_default();
            NameRef {
                kind,
                fqn: to_fqn(fqn),
                start,
                end,
                qualified: text.contains(&b'\\'),
                imported,
            }
        })
        .collect();

    let DocRefs {
        names: doc_names,
        members: doc_members,
        annotations: annotation_names,
    } = doc_names(program, &collector.names);

    let mut keys: HashSet<u64> = HashSet::new();
    for name in names.iter().chain(&doc_names) {
        let fallback = !name.qualified && !name.imported;
        let short = name
            .fqn
            .rsplit_once('\\')
            .map(|(_, short)| short.to_string());
        let key = |fqn: String| match name.kind {
            NameKind::ClassLike => RefKey::Class(fqn),
            NameKind::Function => RefKey::Function(fqn),
            NameKind::Constant => RefKey::Constant(fqn),
        };
        keys.insert(key(name.fqn.clone()).hash());
        // Unqualified functions and constants may reach the global one.
        if fallback
            && name.kind != NameKind::ClassLike
            && let Some(short) = short
        {
            keys.insert(key(short).hash());
        }
    }
    for declaration in &declarations {
        let key = match declaration.kind {
            SymbolKind::Function => RefKey::Function(declaration.fqn.clone()),
            SymbolKind::Constant => RefKey::Constant(declaration.fqn.clone()),
            _ => RefKey::Class(declaration.fqn.clone()),
        };
        keys.insert(key.hash());
        for member in declaration.class.iter().flat_map(|c| c.members.iter()) {
            keys.insert(RefKey::Member(member.name.clone()).hash());
        }
    }
    for member in &doc_members {
        keys.insert(RefKey::Member(member.name.clone()).hash());
    }
    for name in &annotation_names {
        keys.insert(RefKey::Class(name.fqn.clone()).hash());
        if !name.qualified && !name.imported {
            let short = name.fqn.rsplit('\\').next().unwrap_or(&name.fqn);
            keys.insert(RefKey::Annotation(short.to_string()).hash());
        }
    }
    let accessed = collector.member_names.iter();
    for name in accessed.chain(static_refs.iter().map(|r: &StaticRef| &r.name)) {
        keys.insert(RefKey::Member(name.clone()).hash());
    }
    let mut ref_keys: Vec<u64> = keys.into_iter().collect();
    ref_keys.sort_unstable();
    let mut guarded = std::mem::take(&mut collector.guard_names);
    guarded.extend(
        collector
            .guard_classes
            .iter()
            .filter_map(|&start| fqn_at(start))
            .map(|fqn| fqn.to_ascii_lowercase()),
    );
    guarded.sort();
    guarded.dedup();

    FileSummary {
        declarations,
        names,
        doc_names,
        doc_members,
        annotation_names,
        has_parse_errors: program.has_errors(),
        file_names: collector.names,
        static_refs,
        keywords: collector.keywords,
        class_ranges,
        ref_keys,
        guarded,
        unchecked: std::mem::take(&mut collector.unchecked),
        hints: std::mem::take(&mut collector.hints),
        closures: std::mem::take(&mut collector.closures),
    }
}

/// Whether a docblock declares members by tag: `@method` or `@mixin`.
fn has_magic_tags(doc: &str) -> bool {
    ["@method", "@mixin"].iter().any(|tag| {
        doc.match_indices(tag).any(|(i, _)| {
            doc[i + tag.len()..]
                .chars()
                .next()
                .is_none_or(char::is_whitespace)
        })
    })
}

/// Whether `member` is `__call` or `__callStatic`.
fn handles_calls(member: &Member) -> bool {
    member.kind == MemberKind::Method
        && (member.name.eq_ignore_ascii_case("__call")
            || member.name.eq_ignore_ascii_case("__callStatic"))
}

fn to_fqn(name: &[u8]) -> String {
    let name = name.strip_prefix(b"\\").unwrap_or(name);
    String::from_utf8_lossy(name).into_owned()
}

/// PHPDoc wins when it names a class; otherwise the declared type, when it names
/// anything navigable.
pub(crate) fn pick_type(doc: Option<TypeExpr>, declared: Option<TypeExpr>) -> Option<TypeExpr> {
    match doc {
        Some(doc) if names_a_class(&doc) => Some(doc),
        _ => declared.filter(|ty| *ty != TypeExpr::Other),
    }
}

fn names_a_class(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Named(_)
        | TypeExpr::Tagged { .. }
        | TypeExpr::Generic { .. }
        | TypeExpr::SelfType
        | TypeExpr::StaticType
        | TypeExpr::ParentType => true,
        TypeExpr::ArrayOf(element) | TypeExpr::ClassString(element) => names_a_class(element),
        TypeExpr::Union(members) => members.iter().any(names_a_class),
        TypeExpr::Scalar(_) | TypeExpr::Other => false,
    }
}

/// A declaration found during the walk, before its names are resolved.
struct Pending {
    kind: SymbolKind,
    start: u32,
    end: u32,
    /// Class-likes: the whole declaration.
    range: Option<(u32, u32)>,
    /// The whole declaration, for every kind.
    span: (u32, u32),
    /// Set when the resolver does not know the name, as for `define()`.
    fqn: Option<String>,
    signature: String,
    doc: Option<String>,
    returns: Option<TypeExpr>,
    shape: Option<PendingShape>,
    /// Functions: their attributes.
    attributes: Vec<PendingMeta>,
}

/// A class-like's members, the offsets of its trait names, its trait aliases, and
/// its method attributes by the offset of the method name.
type MembersFound = (
    Vec<Member>,
    Vec<u32>,
    Vec<String>,
    Vec<(u32, Vec<PendingMeta>)>,
    // The generic traits that `@use` docblocks give arguments.
    Vec<(String, Vec<TypeExpr>)>,
);

/// Parent, interfaces and traits as offsets of names the resolver resolved.
struct PendingShape {
    parent: Option<u32>,
    interfaces: Vec<u32>,
    /// Interfaces PHP gives the class-like without naming them, after `interfaces`.
    implicit: &'static [&'static str],
    traits: Vec<u32>,
    aliases: Vec<String>,
    members: Vec<Member>,
    is_abstract: bool,
    attributes: Vec<PendingMeta>,
    annotations: Vec<Metadata>,
    /// Method attributes, by the offset of the method name.
    member_attributes: Vec<(u32, Vec<PendingMeta>)>,
    templates: Vec<String>,
    generic_supertypes: Vec<(String, Vec<TypeExpr>)>,
}

/// An attribute before names are resolved: offsets of the names the resolver knows.
struct PendingMeta {
    name: u32,
    end: u32,
    span: (u32, u32),
    arguments: Vec<(Option<String>, PendingValue)>,
}

enum PendingValue {
    Ready(MetaValue),
    /// `X::class`: the offset of `X`.
    Class(u32),
    List(Vec<(Option<String>, PendingValue)>),
    Nested(PendingMeta),
}

impl PendingMeta {
    fn resolve(self, fqn_at: &dyn Fn(u32) -> Option<String>) -> Option<Metadata> {
        Some(Metadata {
            name: fqn_at(self.name)?,
            arguments: self
                .arguments
                .into_iter()
                .map(|(key, value)| (key, value.resolve(fqn_at)))
                .collect(),
            start: self.name,
            end: self.end,
            span: self.span,
            annotation: false,
            explicit: true,
        })
    }
}

impl PendingValue {
    fn resolve(self, fqn_at: &dyn Fn(u32) -> Option<String>) -> MetaValue {
        match self {
            PendingValue::Ready(value) => value,
            PendingValue::Class(offset) => {
                fqn_at(offset).map_or(MetaValue::Other, MetaValue::Class)
            }
            PendingValue::List(entries) => MetaValue::List(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, value.resolve(fqn_at)))
                    .collect(),
            ),
            PendingValue::Nested(meta) => meta
                .resolve(fqn_at)
                .map_or(MetaValue::Other, |m| MetaValue::Nested(Box::new(m))),
        }
    }
}

/// The class of a static access before names are resolved: the offset of a name the
/// resolver resolved, or a keyword.
enum PendingSel {
    Named(u32),
    Keyword(ClassSel),
}

/// Where a class-like declaration's parts are, as offsets.
struct ClassHead {
    kind: SymbolKind,
    is_abstract: bool,
    node_start: u32,
    node_end: u32,
    head_start: u32,
    name: (u32, u32),
    left_brace: u32,
    parent: Option<u32>,
    interfaces: Vec<u32>,
    implicit: &'static [&'static str],
    attributes: Vec<PendingMeta>,
}

struct Collector<'a> {
    source: &'a [u8],
    program: &'a Program<'a>,
    names: FileNames,
    pending: Vec<Pending>,
    functions: HashSet<u32>,
    /// Names checked by `*_exists` and `defined` calls, and offsets of the class
    /// names in `X::class` arguments to those calls.
    guard_names: Vec<String>,
    guard_classes: Vec<u32>,
    unchecked: Vec<(u32, u32)>,
    hints: Vec<(u32, u32)>,
    closures: Vec<(u32, u32)>,
    constants: HashSet<u32>,
    /// Namespace names, which the resolver records but which name no symbol.
    not_refs: HashSet<u32>,
    static_refs: Vec<(MemberKind, String, u32, u32, PendingSel)>,
    keywords: Vec<KeywordRef>,
    anonymous: Vec<PendingAnonymous>,
    /// Names of members accessed with `->` or `?->`.
    member_names: HashSet<String>,
}

/// An anonymous class: its range, the offset of its parent's name, and its members.
struct PendingAnonymous {
    start: u32,
    end: u32,
    parent: Option<u32>,
    members: Vec<Member>,
}

impl Collector<'_> {
    /// Records the name a guard call checks: the class, function or constant of the
    /// `*_exists` functions and `defined`, the method of `method_exists`, and the
    /// function or method of `is_callable`, given by position or by name.
    fn guard(&mut self, function: &[u8], call: &FunctionCall<'_>) {
        let function = function.strip_prefix(b"\\").unwrap_or(function);
        let (position, parameter): (usize, &[u8]) = match function.to_ascii_lowercase().as_slice() {
            b"class_exists" => (0, b"class"),
            b"interface_exists" => (0, b"interface"),
            b"trait_exists" => (0, b"trait"),
            b"enum_exists" => (0, b"enum"),
            b"function_exists" => (0, b"function"),
            b"defined" => (0, b"constant_name"),
            b"method_exists" => (1, b"method"),
            b"is_callable" => (0, b"value"),
            _ => return,
        };
        let value =
            call.argument_list.arguments.iter().enumerate().find_map(
                |(i, argument)| match argument {
                    Argument::Positional(argument) if i == position => Some(argument.value),
                    Argument::Named(argument)
                        if argument.name.value.eq_ignore_ascii_case(parameter) =>
                    {
                        Some(argument.value)
                    }
                    _ => None,
                },
            );
        if let Some(value) = value {
            self.guard_value(value);
        }
    }

    fn guard_value(&mut self, value: &Expression<'_>) {
        match value {
            Expression::Literal(Literal::String(literal)) => {
                if let Some(value) = literal.value {
                    let name = String::from_utf8_lossy(value);
                    self.guard_names
                        .push(name.trim_start_matches('\\').to_ascii_lowercase());
                }
            }
            Expression::Access(Access::ClassConstant(access)) => {
                let is_class = matches!(&access.constant,
                    ClassLikeConstantSelector::Identifier(name) if name.value.eq_ignore_ascii_case(b"class"));
                if let (true, Expression::Identifier(class)) = (is_class, access.class) {
                    self.guard_classes.push(class.span().start.offset);
                }
            }
            // `[$object, 'method']` callables name the method last.
            Expression::Array(array) => self.guard_callable(array.elements.iter()),
            Expression::LegacyArray(array) => self.guard_callable(array.elements.iter()),
            _ => {}
        }
    }

    /// The attributes of a declaration, with the names left to resolve.
    fn attributes(&self, lists: &Sequence<'_, AttributeList<'_>>) -> Vec<PendingMeta> {
        lists
            .iter()
            .flat_map(|list| list.attributes.iter())
            .map(|attribute| {
                let arguments = attribute
                    .argument_list
                    .iter()
                    .flat_map(|list| list.arguments.iter())
                    .filter_map(|argument| match argument {
                        PartialArgument::Positional(a) => Some((None, self.meta_value(a.value))),
                        PartialArgument::Named(a) => Some((
                            Some(String::from_utf8_lossy(a.name.value).into_owned()),
                            self.meta_value(a.value),
                        )),
                        _ => None,
                    })
                    .collect();
                let name = attribute.name.span();
                let span = attribute.span();
                PendingMeta {
                    name: name.start.offset,
                    end: name.end.offset,
                    span: (span.start.offset, span.end.offset),
                    arguments,
                }
            })
            .collect()
    }

    fn meta_value(&self, expression: &Expression<'_>) -> PendingValue {
        match expression {
            Expression::Literal(Literal::String(literal)) => {
                PendingValue::Ready(literal.value.map_or(MetaValue::Other, |v| {
                    MetaValue::String(String::from_utf8_lossy(v).into_owned())
                }))
            }
            Expression::Literal(Literal::True(_)) => PendingValue::Ready(MetaValue::Bool(true)),
            Expression::Literal(Literal::False(_)) => PendingValue::Ready(MetaValue::Bool(false)),
            Expression::Literal(Literal::Integer(_) | Literal::Float(_)) => {
                let span = expression.span();
                PendingValue::Ready(MetaValue::Number(
                    self.text(span.start.offset, span.end.offset),
                ))
            }
            Expression::Access(Access::ClassConstant(access)) => {
                let is_class = matches!(&access.constant,
                    ClassLikeConstantSelector::Identifier(name) if name.value.eq_ignore_ascii_case(b"class"));
                match (is_class, access.class) {
                    (true, Expression::Identifier(class)) => {
                        PendingValue::Class(class.span().start.offset)
                    }
                    _ => PendingValue::Ready(MetaValue::Other),
                }
            }
            Expression::Array(array) => self.meta_list(array.elements.iter()),
            Expression::LegacyArray(array) => self.meta_list(array.elements.iter()),
            Expression::Instantiation(instantiation) => {
                let Expression::Identifier(class) = instantiation.class else {
                    return PendingValue::Ready(MetaValue::Other);
                };
                let arguments = instantiation
                    .argument_list
                    .iter()
                    .flat_map(|list| list.arguments.iter())
                    .map(|argument| match argument {
                        Argument::Positional(a) => (None, self.meta_value(a.value)),
                        Argument::Named(a) => (
                            Some(String::from_utf8_lossy(a.name.value).into_owned()),
                            self.meta_value(a.value),
                        ),
                    })
                    .collect();
                let name = class.span();
                let span = instantiation.span();
                PendingValue::Nested(PendingMeta {
                    name: name.start.offset,
                    end: name.end.offset,
                    span: (span.start.offset, span.end.offset),
                    arguments,
                })
            }
            _ => PendingValue::Ready(MetaValue::Other),
        }
    }

    fn meta_list<'e>(&self, elements: impl Iterator<Item = &'e ArrayElement<'e>>) -> PendingValue {
        PendingValue::List(
            elements
                .filter_map(|element| match element {
                    ArrayElement::KeyValue(entry) => {
                        let key = match entry.key {
                            Expression::Literal(Literal::String(literal)) => literal
                                .value
                                .map(|v| String::from_utf8_lossy(v).into_owned()),
                            Expression::Literal(Literal::Integer(_)) => {
                                let span = entry.key.span();
                                Some(self.text(span.start.offset, span.end.offset))
                            }
                            _ => None,
                        };
                        Some((key, self.meta_value(entry.value)))
                    }
                    ArrayElement::Value(entry) => Some((None, self.meta_value(entry.value))),
                    _ => None,
                })
                .collect(),
        )
    }

    /// Records `class_alias(X::class, 'Alias')` as a class-like named `Alias`, with no
    /// shape: its members are unknown.
    fn class_alias(&mut self, function: &[u8], call: &FunctionCall<'_>) {
        let function = function.strip_prefix(b"\\").unwrap_or(function);
        if !function.eq_ignore_ascii_case(b"class_alias") {
            return;
        }
        let alias = call
            .argument_list
            .arguments
            .iter()
            .enumerate()
            .find_map(|(i, argument)| match argument {
                Argument::Positional(argument) if i == 1 => Some(argument.value),
                Argument::Named(argument) if argument.name.value.eq_ignore_ascii_case(b"alias") => {
                    Some(argument.value)
                }
                _ => None,
            });
        let Some(Expression::Literal(Literal::String(literal))) = alias else {
            return;
        };
        let Some(value) = literal.value else {
            return;
        };
        let name = String::from_utf8_lossy(value)
            .trim_start_matches('\\')
            .to_string();
        self.pending.push(Pending {
            attributes: Vec::new(),
            kind: SymbolKind::Class,
            range: None,
            span: (call.span().start.offset, call.span().end.offset),
            start: literal.span.start.offset + 1,
            end: literal.span.end.offset.saturating_sub(1),
            signature: format!("class_alias('{name}')"),
            fqn: Some(name),
            doc: None,
            returns: None,
            shape: None,
        });
    }

    fn guard_callable<'e>(&mut self, elements: impl Iterator<Item = &'e ArrayElement<'e>>) {
        if let Some(ArrayElement::Value(method)) = elements.last() {
            self.guard_value(method.value);
        }
    }

    /// The class an expression before `::` or after `new` names, recording `self`,
    /// `static` and `parent` as keyword references. Dynamic classes give nothing.
    fn class_sel(&mut self, expression: &Expression<'_>) -> Option<PendingSel> {
        let (keyword, sel) = match expression {
            Expression::Identifier(identifier) => {
                return Some(PendingSel::Named(identifier.span().start.offset));
            }
            Expression::Self_(keyword) => (keyword, ClassSel::SelfKw),
            Expression::Static(keyword) => (keyword, ClassSel::StaticKw),
            Expression::Parent(keyword) => (keyword, ClassSel::ParentKw),
            _ => return None,
        };
        self.keywords.push(KeywordRef {
            start: keyword.span.start.offset,
            end: keyword.span.end.offset,
            class: sel.clone(),
        });
        Some(PendingSel::Keyword(sel))
    }

    fn static_member(
        &mut self,
        class: &Expression<'_>,
        kind: MemberKind,
        name: &[u8],
        start: u32,
        end: u32,
    ) {
        let Some(sel) = self.class_sel(class) else {
            return;
        };
        let name = String::from_utf8_lossy(name)
            .trim_start_matches('$')
            .to_string();
        if name.eq_ignore_ascii_case("class") && kind == MemberKind::Constant {
            return;
        }
        self.static_refs.push((kind, name, start, end, sel));
    }

    fn member_name(&mut self, selector: &ClassLikeMemberSelector<'_>) {
        if let ClassLikeMemberSelector::Identifier(name) = selector {
            self.member_names
                .insert(String::from_utf8_lossy(name.value).into_owned());
        }
    }

    fn method_selector(&mut self, class: &Expression<'_>, selector: &ClassLikeMemberSelector<'_>) {
        if let ClassLikeMemberSelector::Identifier(name) = selector {
            self.static_member(
                class,
                MemberKind::Method,
                name.value,
                name.span.start.offset,
                name.span.end.offset,
            );
        } else {
            self.class_sel(class);
        }
    }

    fn text(&self, start: u32, end: u32) -> String {
        let bytes = self
            .source
            .get(start as usize..end as usize)
            .unwrap_or_default();
        collapse(&String::from_utf8_lossy(bytes))
    }

    fn doc(&self, start: u32) -> Option<String> {
        get_docblock_before_position(self.program.trivia.as_slice(), start)
            .map(|trivia| String::from_utf8_lossy(trivia.value).into_owned())
    }

    fn hint_type(&self, hint: &Hint<'_>) -> TypeExpr {
        let span = hint.span();
        let text = self.text(span.start.offset, span.end.offset);
        parse_type(&text, &self.names.at(span.start.offset))
    }

    fn doc_tags(&self, doc: Option<&str>, offset: u32) -> crate::phpdoc::DocBlock {
        doc.map(|doc| parse_docblock(doc, &self.names.at(offset)))
            .unwrap_or_default()
    }

    fn class_like(&mut self, head: ClassHead, members: &Sequence<'_, ClassLikeMember<'_>>) {
        let (members, traits, aliases, member_attributes, generic_uses) =
            self.members(members, head.kind == SymbolKind::Interface);
        let annotations =
            get_docblock_before_position(self.program.trivia.as_slice(), head.node_start)
                .map(|trivia| {
                    let offset = trivia.span.start.offset;
                    let doc = String::from_utf8_lossy(trivia.value);
                    metadata::annotations(&doc, offset, &self.names.at(offset))
                })
                .unwrap_or_default();
        let doc = self.doc(head.node_start);
        let tags = self.doc_tags(doc.as_deref(), head.node_start);
        self.pending.push(Pending {
            attributes: Vec::new(),
            kind: head.kind,
            start: head.name.0,
            end: head.name.1,
            range: Some((head.node_start, head.node_end)),
            span: (head.node_start, head.node_end),
            fqn: None,
            signature: self.text(head.head_start, head.left_brace),
            doc,
            returns: None,
            shape: Some(PendingShape {
                parent: head.parent,
                interfaces: head.interfaces,
                implicit: head.implicit,
                traits,
                aliases,
                members,
                is_abstract: head.is_abstract || head.kind == SymbolKind::Interface,
                attributes: head.attributes,
                annotations,
                member_attributes,
                templates: tags.templates,
                generic_supertypes: tags
                    .generic_supertypes
                    .into_iter()
                    .chain(generic_uses)
                    .collect(),
            }),
        });
    }

    fn members(
        &self,
        members: &Sequence<'_, ClassLikeMember<'_>>,
        in_interface: bool,
    ) -> MembersFound {
        let mut out = Vec::new();
        let mut traits = Vec::new();
        let mut aliases = Vec::new();
        let mut attributes = Vec::new();
        let mut generic_uses = Vec::new();
        for member in members.iter() {
            match member {
                ClassLikeMember::TraitUse(trait_use) => {
                    traits.extend(trait_use.trait_names.iter().map(|n| n.span().start.offset));
                    let start = trait_use.span().start.offset;
                    generic_uses.extend(
                        self.doc_tags(self.doc(start).as_deref(), start)
                            .generic_supertypes,
                    );
                    if let TraitUseSpecification::Concrete(specification) = &trait_use.specification
                    {
                        for adaptation in specification.adaptations.iter() {
                            if let TraitUseAdaptation::Alias(alias) = adaptation
                                && let Some(name) = &alias.alias
                            {
                                aliases
                                    .push(String::from_utf8_lossy(name.value).to_ascii_lowercase());
                            }
                        }
                    }
                }
                ClassLikeMember::Constant(constant) => {
                    let start = constant.span().start.offset;
                    let doc = self.doc(start);
                    for item in constant.items.iter() {
                        let name = String::from_utf8_lossy(item.name.value).into_owned();
                        out.push(Member {
                            metadata: Vec::new(),
                            kind: MemberKind::Constant,
                            signature: format!("const {name}"),
                            name,
                            is_static: true,
                            name_start: item.name.span.start.offset,
                            name_end: item.name.span.end.offset,
                            doc: doc.clone(),
                            ty: None,
                            params: Vec::new(),
                            hints: Vec::new(),
                            is_abstract: false,
                            is_private: false,
                            templates: Vec::new(),
                            range: (start, constant.span().end.offset),
                        });
                    }
                }
                ClassLikeMember::Property(property) => self.property(property, &mut out),
                ClassLikeMember::EnumCase(case) => {
                    let name = match &case.item {
                        EnumCaseItem::Unit(item) => &item.name,
                        EnumCaseItem::Backed(item) => &item.name,
                    };
                    let text = String::from_utf8_lossy(name.value).into_owned();
                    out.push(Member {
                        metadata: Vec::new(),
                        kind: MemberKind::EnumCase,
                        signature: format!("case {text}"),
                        name: text,
                        is_static: true,
                        name_start: name.span.start.offset,
                        name_end: name.span.end.offset,
                        doc: self.doc(case.span().start.offset),
                        ty: None,
                        params: Vec::new(),
                        hints: Vec::new(),
                        is_abstract: false,
                        is_private: false,
                        templates: Vec::new(),
                        range: (case.span().start.offset, case.span().end.offset),
                    });
                }
                ClassLikeMember::Method(method) => {
                    if !method.attribute_lists.is_empty() {
                        attributes.push((
                            method.name.span.start.offset,
                            self.attributes(&method.attribute_lists),
                        ));
                    }
                    self.method(method, in_interface, &mut out);
                }
            }
        }
        (out, traits, aliases, attributes, generic_uses)
    }

    fn property(&self, property: &Property<'_>, out: &mut Vec<Member>) {
        let (modifiers, hint, items): (_, _, Vec<&PropertyItem<'_>>) = match property {
            Property::Plain(plain) => (
                &plain.modifiers,
                plain.hint.as_ref(),
                plain.items.iter().collect(),
            ),
            Property::Hooked(hooked) => {
                (&hooked.modifiers, hooked.hint.as_ref(), vec![&hooked.item])
            }
        };
        let start = property.span().start.offset;
        let doc = self.doc(start);
        let tags = self.doc_tags(doc.as_deref(), start);
        let declared = hint.map(|h| self.hint_type(h));
        let first_item = items
            .first()
            .map(|item| variable_of(item).span.start.offset);
        let prefix = first_item.map_or_else(String::new, |end| {
            let head = modifiers
                .first()
                .map(|m| m.span().start.offset)
                .unwrap_or(start);
            self.text(head, end)
        });
        for item in items {
            let variable = variable_of(item);
            let name = String::from_utf8_lossy(variable.name)
                .trim_start_matches('$')
                .to_string();
            let doc_type = tags
                .var
                .iter()
                .find(|(var, _)| var.as_deref().is_none_or(|v| v == name))
                .map(|(_, ty)| ty.clone());
            out.push(Member {
                metadata: Vec::new(),
                kind: MemberKind::Property,
                signature: format!("{prefix} ${name}").trim().to_string(),
                is_static: has_static(modifiers),
                name_start: variable.span.start.offset + 1,
                name_end: variable.span.end.offset,
                doc: doc.clone(),
                ty: pick_type(doc_type, declared.clone()),
                params: Vec::new(),
                hints: Vec::new(),
                is_abstract: false,
                is_private: has_private(modifiers),
                templates: Vec::new(),
                range: (start, property.span().end.offset),
                name,
            });
        }
    }

    fn method(&self, method: &Method<'_>, in_interface: bool, out: &mut Vec<Member>) {
        let start = method.span().start.offset;
        let doc = self.doc(start);
        let tags = self.doc_tags(doc.as_deref(), start);
        let head = method
            .modifiers
            .first()
            .map(|m| m.span().start.offset)
            .unwrap_or(method.function.span.start.offset);
        let end = head_end(&method.parameter_list, method.return_type_hint.as_ref());
        let params = self.params(&method.parameter_list, &tags);
        let name = String::from_utf8_lossy(method.name.value).into_owned();

        if name.eq_ignore_ascii_case("__construct") {
            for (param, (param_name, ty)) in method.parameter_list.parameters.iter().zip(&params) {
                if param.modifiers.is_empty() {
                    continue;
                }
                let head = param.span().start.offset;
                out.push(Member {
                    metadata: Vec::new(),
                    kind: MemberKind::Property,
                    name: param_name.clone(),
                    is_static: false,
                    name_start: param.variable.span.start.offset + 1,
                    name_end: param.variable.span.end.offset,
                    signature: self.text(head, param.variable.span.end.offset),
                    doc: None,
                    ty: ty.clone(),
                    params: Vec::new(),
                    hints: Vec::new(),
                    is_abstract: false,
                    is_private: has_private(&param.modifiers),
                    templates: Vec::new(),
                    range: (param.span().start.offset, param.span().end.offset),
                });
            }
        }

        out.push(Member {
            metadata: Vec::new(),
            kind: MemberKind::Method,
            is_static: has_static(&method.modifiers),
            name_start: method.name.span.start.offset,
            name_end: method.name.span.end.offset,
            signature: self.text(head, end),
            ty: pick_type(
                tags.returns.clone(),
                method
                    .return_type_hint
                    .as_ref()
                    .map(|r| self.hint_type(&r.hint)),
            ),
            doc,
            params,
            hints: method
                .parameter_list
                .parameters
                .iter()
                .map(|param| param.hint.as_ref().map(|h| self.hint_type(h)))
                .collect(),
            is_abstract: in_interface || has_abstract(&method.modifiers),
            is_private: has_private(&method.modifiers),
            templates: tags.templates.clone(),
            range: (start, method.span().end.offset),
            name,
        });
    }

    fn params(
        &self,
        list: &FunctionLikeParameterList<'_>,
        tags: &crate::phpdoc::DocBlock,
    ) -> Vec<(String, Option<TypeExpr>)> {
        list.parameters
            .iter()
            .map(|param| {
                let name = String::from_utf8_lossy(param.variable.name)
                    .trim_start_matches('$')
                    .to_string();
                let doc_type = tags
                    .params
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, ty)| ty.clone());
                let declared = param.hint.as_ref().map(|h| self.hint_type(h));
                let ty = pick_type(doc_type, declared);
                (name, ty)
            })
            .collect()
    }
}

fn variable_of<'a>(item: &'a PropertyItem<'_>) -> &'a mago_syntax::cst::DirectVariable<'a> {
    match item {
        PropertyItem::Abstract(item) => &item.variable,
        PropertyItem::Concrete(item) => &item.variable,
    }
}

fn has_private(modifiers: &Sequence<'_, Modifier<'_>>) -> bool {
    modifiers.iter().any(|m| matches!(m, Modifier::Private(_)))
}

fn has_abstract(modifiers: &Sequence<'_, Modifier<'_>>) -> bool {
    modifiers.iter().any(|m| matches!(m, Modifier::Abstract(_)))
}

fn has_static(modifiers: &Sequence<'_, Modifier<'_>>) -> bool {
    modifiers.iter().any(|m| matches!(m, Modifier::Static(_)))
}

/// Where a function-like head ends: after the return type, or after the parameters.
fn head_end(
    params: &FunctionLikeParameterList<'_>,
    returns: Option<&FunctionLikeReturnTypeHint<'_>>,
) -> u32 {
    returns
        .map(|r| r.span().end.offset)
        .unwrap_or(params.right_parenthesis.end.offset)
}

/// Collapses runs of whitespace to single spaces.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct SyntaxVisitor;

impl<'ast, 'arena> Walker<'ast, 'arena, Collector<'arena>> for SyntaxVisitor {
    fn walk_in_namespace(&self, namespace: &'ast Namespace<'arena>, c: &mut Collector<'arena>) {
        let span = namespace.span();
        let name = namespace
            .name
            .as_ref()
            .map(|name| to_fqn(name.value()))
            .unwrap_or_default();
        if let Some(name) = &namespace.name {
            c.not_refs.insert(name.span().start.offset);
        }
        c.names.scopes.push(NameScope {
            start: span.start.offset,
            end: span.end.offset,
            namespace: name,
            ..NameScope::default()
        });
    }

    fn walk_in_closure(&self, closure: &'ast Closure<'arena>, c: &mut Collector<'arena>) {
        let span = closure.span();
        c.closures.push((span.start.offset, span.end.offset));
    }

    fn walk_in_arrow_function(
        &self,
        arrow: &'ast ArrowFunction<'arena>,
        c: &mut Collector<'arena>,
    ) {
        let span = arrow.span();
        c.closures.push((span.start.offset, span.end.offset));
    }

    fn walk_in_attribute(&self, attribute: &'ast Attribute<'arena>, c: &mut Collector<'arena>) {
        let span = attribute.span();
        c.unchecked.push((span.start.offset, span.end.offset));
    }

    fn walk_in_use(&self, r#use: &'ast Use<'arena>, c: &mut Collector<'arena>) {
        let span = r#use.span();
        c.unchecked.push((span.start.offset, span.end.offset));
        let offset = r#use.span().start.offset;
        let mut imports: Vec<(String, String)> = Vec::new();
        let mut functions: Vec<(String, String)> = Vec::new();
        match &r#use.items {
            UseItems::Sequence(seq) => {
                for item in seq.items.iter() {
                    imports.push(import(
                        item.name.value(),
                        item.alias.as_ref().map(|a| a.identifier.value),
                        None,
                    ));
                }
            }
            UseItems::TypedSequence(seq) => {
                for item in seq.items.iter() {
                    mark_use_item(c, Some(&seq.r#type), item.name.span().start.offset);
                    if matches!(seq.r#type, UseType::Function(_)) {
                        functions.push(import(
                            item.name.value(),
                            item.alias.as_ref().map(|a| a.identifier.value),
                            None,
                        ));
                    }
                }
            }
            UseItems::TypedList(list) => {
                c.not_refs.insert(list.namespace.span().start.offset);
                for item in list.items.iter() {
                    mark_use_item(c, Some(&list.r#type), item.name.span().start.offset);
                    if matches!(list.r#type, UseType::Function(_)) {
                        functions.push(import(
                            item.name.value(),
                            item.alias.as_ref().map(|a| a.identifier.value),
                            Some(list.namespace.value()),
                        ));
                    }
                }
            }
            UseItems::MixedList(list) => {
                c.not_refs.insert(list.namespace.span().start.offset);
                for mixed in list.items.iter() {
                    mark_use_item(
                        c,
                        mixed.r#type.as_ref(),
                        mixed.item.name.span().start.offset,
                    );
                    let item = import(
                        mixed.item.name.value(),
                        mixed.item.alias.as_ref().map(|a| a.identifier.value),
                        Some(list.namespace.value()),
                    );
                    match mixed.r#type {
                        None => imports.push(item),
                        Some(UseType::Function(_)) => functions.push(item),
                        Some(_) => {}
                    }
                }
            }
        }
        if let Some(index) = c.names.scope_index(offset) {
            c.names.scopes[index].imports.extend(imports);
            c.names.scopes[index].function_imports.extend(functions);
        }
    }

    fn walk_in_class(&self, class: &'ast Class<'arena>, c: &mut Collector<'arena>) {
        let head = class
            .modifiers
            .first()
            .map(|m| m.span().start.offset)
            .unwrap_or(class.class.span.start.offset);
        let parent = class
            .extends
            .as_ref()
            .and_then(|e| e.types.iter().next())
            .map(|t| t.span().start.offset);
        let interfaces = class
            .implements
            .as_ref()
            .map(|i| i.types.iter().map(|t| t.span().start.offset).collect())
            .unwrap_or_default();
        c.class_like(
            ClassHead {
                kind: SymbolKind::Class,
                is_abstract: has_abstract(&class.modifiers),
                node_start: class.span().start.offset,
                node_end: class.span().end.offset,
                head_start: head,
                name: (class.name.span.start.offset, class.name.span.end.offset),
                left_brace: class.left_brace.start.offset,
                parent,
                interfaces,
                implicit: &[],
                attributes: c.attributes(&class.attribute_lists),
            },
            &class.members,
        );
    }

    fn walk_in_method_call(&self, call: &'ast MethodCall<'arena>, c: &mut Collector<'arena>) {
        c.member_name(&call.method);
    }

    fn walk_in_null_safe_method_call(
        &self,
        call: &'ast NullSafeMethodCall<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.member_name(&call.method);
    }

    fn walk_in_property_access(
        &self,
        access: &'ast PropertyAccess<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.member_name(&access.property);
    }

    fn walk_in_null_safe_property_access(
        &self,
        access: &'ast NullSafePropertyAccess<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.member_name(&access.property);
    }

    fn walk_in_anonymous_class(
        &self,
        class: &'ast AnonymousClass<'arena>,
        c: &mut Collector<'arena>,
    ) {
        let span = class.span();
        let (members, _, _, _, _) = c.members(&class.members, false);
        c.anonymous.push(PendingAnonymous {
            start: span.start.offset,
            end: span.end.offset,
            parent: class
                .extends
                .as_ref()
                .and_then(|e| e.types.iter().next())
                .map(|t| t.span().start.offset),
            members,
        });
    }

    fn walk_in_interface(&self, interface: &'ast Interface<'arena>, c: &mut Collector<'arena>) {
        let interfaces = interface
            .extends
            .as_ref()
            .map(|e| e.types.iter().map(|t| t.span().start.offset).collect())
            .unwrap_or_default();
        c.class_like(
            ClassHead {
                kind: SymbolKind::Interface,
                is_abstract: false,
                node_start: interface.span().start.offset,
                node_end: interface.span().end.offset,
                head_start: interface.interface.span.start.offset,
                name: (
                    interface.name.span.start.offset,
                    interface.name.span.end.offset,
                ),
                left_brace: interface.left_brace.start.offset,
                parent: None,
                interfaces,
                implicit: &[],
                attributes: c.attributes(&interface.attribute_lists),
            },
            &interface.members,
        );
    }

    fn walk_in_trait(&self, r#trait: &'ast Trait<'arena>, c: &mut Collector<'arena>) {
        c.class_like(
            ClassHead {
                kind: SymbolKind::Trait,
                is_abstract: false,
                node_start: r#trait.span().start.offset,
                node_end: r#trait.span().end.offset,
                head_start: r#trait.r#trait.span.start.offset,
                name: (r#trait.name.span.start.offset, r#trait.name.span.end.offset),
                left_brace: r#trait.left_brace.start.offset,
                parent: None,
                interfaces: Vec::new(),
                implicit: &[],
                attributes: c.attributes(&r#trait.attribute_lists),
            },
            &r#trait.members,
        );
    }

    fn walk_in_enum(&self, r#enum: &'ast Enum<'arena>, c: &mut Collector<'arena>) {
        let interfaces = r#enum
            .implements
            .as_ref()
            .map(|i| i.types.iter().map(|t| t.span().start.offset).collect())
            .unwrap_or_default();
        c.class_like(
            ClassHead {
                kind: SymbolKind::Enum,
                is_abstract: false,
                node_start: r#enum.span().start.offset,
                node_end: r#enum.span().end.offset,
                head_start: r#enum.r#enum.span.start.offset,
                name: (r#enum.name.span.start.offset, r#enum.name.span.end.offset),
                left_brace: r#enum.left_brace.start.offset,
                parent: None,
                interfaces,
                implicit: if r#enum.backing_type_hint.is_some() {
                    &["UnitEnum", "BackedEnum"]
                } else {
                    &["UnitEnum"]
                },
                attributes: c.attributes(&r#enum.attribute_lists),
            },
            &r#enum.members,
        );
    }

    fn walk_in_function(&self, function: &'ast Function<'arena>, c: &mut Collector<'arena>) {
        let start = function.span().start.offset;
        let doc = c.doc(start);
        let tags = c.doc_tags(doc.as_deref(), start);
        let end = head_end(&function.parameter_list, function.return_type_hint.as_ref());
        let returns = pick_type(
            tags.returns,
            function
                .return_type_hint
                .as_ref()
                .map(|r| c.hint_type(&r.hint)),
        );
        c.pending.push(Pending {
            attributes: c.attributes(&function.attribute_lists),
            kind: SymbolKind::Function,
            start: function.name.span.start.offset,
            end: function.name.span.end.offset,
            range: None,
            span: (start, function.span().end.offset),
            fqn: None,
            signature: c.text(function.function.span.start.offset, end),
            doc,
            returns,
            shape: None,
        });
    }

    fn walk_in_constant(&self, constant: &'ast Constant<'arena>, c: &mut Collector<'arena>) {
        let doc = c.doc(constant.span().start.offset);
        for item in constant.items.iter() {
            c.constants.insert(item.name.span.start.offset);
            c.pending.push(Pending {
                attributes: Vec::new(),
                kind: SymbolKind::Constant,
                start: item.name.span.start.offset,
                end: item.name.span.end.offset,
                range: None,
                span: (constant.span().start.offset, constant.span().end.offset),
                fqn: None,
                signature: format!("const {}", String::from_utf8_lossy(item.name.value)),
                doc: doc.clone(),
                returns: None,
                shape: None,
            });
        }
    }

    fn walk_in_function_call(&self, call: &'ast FunctionCall<'arena>, c: &mut Collector<'arena>) {
        let Expression::Identifier(identifier) = call.function else {
            return;
        };
        c.functions.insert(identifier.span().start.offset);
        c.guard(identifier.value(), call);
        c.class_alias(identifier.value(), call);
        if !identifier.value().eq_ignore_ascii_case(b"define") {
            return;
        }
        let first = call.argument_list.arguments.iter().next();
        let Some(Argument::Positional(argument)) = first else {
            return;
        };
        let Expression::Literal(Literal::String(literal)) = argument.value else {
            return;
        };
        let Some(value) = literal.value else {
            return;
        };
        let name = String::from_utf8_lossy(value).into_owned();
        c.pending.push(Pending {
            attributes: Vec::new(),
            kind: SymbolKind::Constant,
            range: None,
            span: (call.span().start.offset, call.span().end.offset),
            start: literal.span.start.offset + 1,
            end: literal.span.end.offset.saturating_sub(1),
            signature: format!("define('{name}')"),
            fqn: Some(name.trim_start_matches('\\').to_string()),
            doc: c.doc(call.span().start.offset),
            returns: None,
            shape: None,
        });
    }

    fn walk_in_function_partial_application(
        &self,
        application: &'ast FunctionPartialApplication<'arena>,
        c: &mut Collector<'arena>,
    ) {
        if let Expression::Identifier(identifier) = application.function {
            c.functions.insert(identifier.span().start.offset);
        }
    }

    fn walk_in_static_method_call(
        &self,
        call: &'ast StaticMethodCall<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.method_selector(call.class, &call.method);
    }

    fn walk_in_static_method_partial_application(
        &self,
        application: &'ast StaticMethodPartialApplication<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.method_selector(application.class, &application.method);
    }

    fn walk_in_class_constant_access(
        &self,
        access: &'ast ClassConstantAccess<'arena>,
        c: &mut Collector<'arena>,
    ) {
        if let ClassLikeConstantSelector::Identifier(name) = &access.constant {
            c.static_member(
                access.class,
                MemberKind::Constant,
                name.value,
                name.span.start.offset,
                name.span.end.offset,
            );
        } else {
            c.class_sel(access.class);
        }
    }

    fn walk_in_static_property_access(
        &self,
        access: &'ast StaticPropertyAccess<'arena>,
        c: &mut Collector<'arena>,
    ) {
        if let Variable::Direct(variable) = &access.property {
            c.static_member(
                access.class,
                MemberKind::Property,
                variable.name,
                variable.span.start.offset,
                variable.span.end.offset,
            );
        } else {
            c.class_sel(access.class);
        }
    }

    fn walk_in_instantiation(
        &self,
        instantiation: &'ast Instantiation<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.class_sel(instantiation.class);
    }

    fn walk_in_hint(&self, hint: &'ast Hint<'arena>, c: &mut Collector<'arena>) {
        let span = hint.span();
        c.hints.push((span.start.offset, span.end.offset));
        let (keyword, class) = match hint {
            Hint::Self_(keyword) => (keyword, ClassSel::SelfKw),
            Hint::Static(keyword) => (keyword, ClassSel::StaticKw),
            Hint::Parent(keyword) => (keyword, ClassSel::ParentKw),
            _ => return,
        };
        c.keywords.push(KeywordRef {
            start: keyword.span.start.offset,
            end: keyword.span.end.offset,
            class,
        });
    }

    fn walk_in_constant_access(
        &self,
        access: &'ast ConstantAccess<'arena>,
        c: &mut Collector<'arena>,
    ) {
        c.constants.insert(access.name.span().start.offset);
    }
}

/// An import as (lowercased alias, fully qualified name).
fn import(name: &[u8], alias: Option<&[u8]>, prefix: Option<&[u8]>) -> (String, String) {
    let name = to_fqn(name);
    let fqn = match prefix {
        Some(prefix) => format!("{}\\{name}", to_fqn(prefix)),
        None => name,
    };
    let alias = match alias {
        Some(alias) => String::from_utf8_lossy(alias).into_owned(),
        None => fqn.rsplit('\\').next().unwrap_or(&fqn).to_string(),
    };
    (alias.to_ascii_lowercase(), fqn)
}

fn mark_use_item(c: &mut Collector<'_>, use_type: Option<&UseType<'_>>, start: u32) {
    match use_type {
        Some(UseType::Function(_)) => {
            c.functions.insert(start);
        }
        Some(UseType::Const(_)) => {
            c.constants.insert(start);
        }
        None => {}
    }
}
