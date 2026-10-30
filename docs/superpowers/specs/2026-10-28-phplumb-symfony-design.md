# PHPlumb Symfony design

PHPlumb gains a Symfony module and closes the core PHP gaps that stop navigation on Symfony code. The core work also improves Drupal projects.

Status: draft, 2026-10-28. It extends the PHPlumb design (`2026-10-26-php-navigator-design.md`), which listed Symfony as a later module. Every goal, anti-goal and constraint of that design still applies unless this document says otherwise.

## Why

The user reviews PHP code written by AI agents. Besides Drupal, they now review Symfony projects and want the same navigation: from a called method to its implementation, from a method to the places that use it, and from configuration to code.

The Symfony projects the user works on rely on autowiring and attributes, with little YAML. Services are discovered from `src/` with `resource:`, routes are `#[Route]` attributes, and message handlers are `#[AsMessageHandler]` attributes. The links that matter are therefore mostly strings and attributes in PHP and Twig, not YAML definitions.

A survey of the reference project showed that the core already resolves 95.1% of method calls. The calls it misses fail because of core PHP gaps, not because of Symfony. The user chose to fix those gaps as part of this work.

## Goals

### Core PHP

These apply to every project, Drupal included:

1. **Built-in class members.** Methods, properties and constants of PHP's built-in classes and interfaces resolve, for example `$exception->getMessage()`, `$date->format()`, `$file->getPathname()` and `\Fiber::suspend()`. This includes members that project classes inherit from a built-in class, such as `getMessage()` on a project exception extending `\RuntimeException`. Go to definition opens the member's declaration in the stubs, and the member's return type lets navigation continue on the next call. Go to definition on a built-in class, function or constant name opens its stub declaration too, so the Drupal case that expects nothing for `count()` changes to expect the stub. Stub declarations are targets of go to definition and sources of types only: references, implementations, type and call hierarchies, and symbol search do not list them. A built-in name resolves to its stub alone, even when a vendor polyfill also declares it.
2. **Enums.** The implicit members of enums resolve: `X::from()`, `X::tryFrom()`, `X::cases()`, `->value` and `->name`. An enum case access such as `Role::PlatformAdmin` has the enum's type, so `Role::PlatformAdmin->label()` resolves to the enum's method. `from()` and `tryFrom()` return the enum, so `Severity::from($x)->label()` resolves too.
3. **Element types.** The type of an element flows from a typed array or collection to the code that receives it:
   - untyped closure and arrow function parameters passed to built-in array functions such as `array_map`, `array_filter`, `usort` and `array_reduce`;
   - `foreach` over arrays and over generic collections such as Doctrine's `Collection<int, X>`, which declare their element type through `@template` and `IteratorAggregate`;
   - array access on typed arrays, such as `$concepts[$id]->title()` where `$concepts` is `array<string, Concept>`.

### Symfony module

The module is optional and configurable like the Drupal module:

1. **Routes.** Navigate from a route name to its route definition and its controller method. Route names appear in `redirectToRoute()` and `generateUrl()` of `AbstractController`, in `UrlGeneratorInterface::generate()`, and in Twig `path()` and `url()`. Routes come from `#[Route]` attributes on project classes and from YAML route files under `config/routes`. Route names follow the naming rules of the Symfony version installed in the project, including class-level prefixes and generated default names.
2. **Templates.** Navigate from a template name to its Twig file. Names appear in `render()`, `renderView()`, `#[Template]`, and in Twig tags and functions: `extends`, `include`, `embed`, `import`, `from`, `use`, `include()` and `source()`. Template namespaces such as `@Identity` come from `twig.paths` in `config/packages/twig.yaml`; names without a namespace resolve against the default template directory and the paths without a namespace.
3. **Messenger.** Navigate from a `dispatch()` call on a message bus to every handler of the message, and from a handler to every place that dispatches its message. Handlers are classes or methods with `#[AsMessageHandler]`; the handled class comes from the attribute's `handles` argument or from the type of the handler's first parameter. A handler of a parent class or interface of the message also handles it, as in Symfony.
4. **Parameters.** Navigate from a parameter name to its definition under `parameters:` in the project's YAML configuration. Names appear in `#[Autowire(param: 'name')]`, `#[Autowire('%name%')]`, `getParameter('name')` and `%name%` inside YAML values. Parameters the kernel sets itself, such as `kernel.project_dir`, have no target.
5. **YAML navigation.** In `config/services*.yaml` and `config/packages/*.yaml`, Cmd+click on a class name opens the class, and on `@id` opens the service definition, or the class when the id is a class name. This covers service keys, `class:`, `!service` blocks, aliases, and the message classes under `framework.messenger.routing`. `#[Autowire(service: 'id')]` leads to the same targets.
6. **Doctrine mapping.** In `*.orm.xml` mapping files, Cmd+click on an entity, embeddable or target class opens the class, and on a field, association or id name opens the property. `getRepository(X::class)` returns the repository class that the mapping, either XML `repository-class` or the `#[ORM\Entity(repositoryClass:)]` attribute, declares for `X`.
7. **Broken Symfony references.** Flag route names, template names and parameter names that are defined nowhere, following the false-alarm rules below.

## Non-goals

- Symfony events and event subscribers. They may come later.
- Environment variables (`%env(NAME)%`, `#[Autowire(env:)]`) and `.env` files.
- Diagnostics for service ids. Service and parameter definitions from PHP and XML configuration, the project's and the bundles', are read for navigation only and never make a name certain.
- Navigation into Twig expressions: variables, attribute access such as `{{ user.name }}`, filters and functions.
- Reading the compiled container or router from `var/cache`. The module reads source files only, like the Drupal module.
- Typing the result of `$container->get('id')` in Symfony code. Modern Symfony code uses constructor injection.
- Laravel.

## Anti-goals

The anti-goals of the PHPlumb design hold: no wrong jumps, no false alarms, no heavy editor. In this work they mean:

- **No wrong element types.** An element type comes only from a declared type: a native type, a PHPDoc type or a generic template. When the array or collection has no declared element type, the parameter stays unknown and its calls resolve to nothing.
- **No duplicate built-in targets.** The stubs declare some members more than once, one per PHP version. Each built-in member has exactly one target: the declaration for PHP 8.5.
- **No false Symfony alarms.** A broken route, template or parameter is flagged only when every source that could define it was read. A route imported from a source the module cannot read, a template in a namespace the module does not know, or a parameter set by PHP code with a computed name keeps the diagnostic silent.
- **No cross-talk between modules.** When the Drupal and Symfony modules are both active, neither reads the other project's files as its own. The reference project contains a Drupal codebase in a subdirectory, and the Drupal module reads `#[Route]` attributes from every indexed class today, so without this rule Symfony controllers would appear as Drupal routes.

## Reference project and success criteria

The Symfony reference project is an internal Symfony 8.1 application at `<symfony-reference-project>`. It is used read-only and is never modified or committed. Its `src` holds 713 PHP files in a bounded-context layout, with Doctrine XML mappings and templates under each context. Cases that point into it stay outside the repository, like the Drupal cases.

Measured on 2026-10-28 with the current engine, `src` has 13,211 member accesses and the engine resolves 12,568 (95.1%). The 643 unresolved accesses fall into three causes:

| Cause                                   | Unresolved | Examples                                                                                                 |
| --------------------------------------- | ---------- | -------------------------------------------------------------------------------------------------------- |
| Members of built-in classes             | about 230  | `$exception->getMessage()`, `$date->format()`                                                            |
| Implicit enum members and enum cases    | about 190  | `Severity::from()`, `Risk::cases()`, `$status->value`, `Role::PlatformAdmin->label()`                    |
| Element types of arrays and collections | about 220  | `static fn ($defect) => $defect->id`, `foreach ($this->failurePatterns as $pattern)` over a `Collection` |

The Symfony features have this much use in the reference project:

| Feature                                                   | Uses                 |
| --------------------------------------------------------- | -------------------- |
| `#[Route]` attributes                                     | 194, 162 with a name |
| `redirectToRoute()` calls                                 | 114                  |
| `path()` and `url()` in Twig with a literal route name    | 266                  |
| `render()` calls                                          | 69                   |
| Twig templates                                            | 106                  |
| Twig `extends`, `include`, `embed`, `import`, `from` tags | 176                  |
| Twig `include()` calls                                    | 58                   |
| `#[AsMessageHandler]` attributes                          | 30                   |
| `dispatch()` calls                                        | 28                   |
| `#[Autowire(param:)]` attributes                          | 51                   |
| Doctrine `*.orm.xml` mapping files                        | 30                   |

Success targets:

- **Coverage** of member accesses in the Symfony reference project's `src` reaches at least 98%.
- **No Drupal regression.** On the Drupal reference project, coverage in `web/modules/custom` stays at or above 42,455 of 46,204 accesses, every Drupal case passes, and the speed and memory targets of the PHPlumb design hold with the stubs indexed.
- **Correctness cases** cover each Symfony goal and each core goal, with exact expected targets, including expected "nothing" results for undeclared element types.
- **Diagnostics sweep.** A `bench --diagnostics src` run on the Symfony reference project reports no false alarm, checked by hand as was done for Drupal.

## Constraints

- **The stubs are JetBrains phpstorm-stubs**, the same tag as `builtins.txt` (`v2026.2`), under the Apache License 2.0 with its notice kept. Go to definition must open a real file, so the stub files exist on disk at run time.
- **The modules share code, not each other.** Code both modules need, such as the positioned YAML parser, reading `#[Route]` attributes, and the `is_a` ancestry check, moves into shared internal modules. The Drupal and Symfony modules stay separate implementations of `Framework`, and neither depends on the other being active.
- **Each module reads its own project.** A module's project is the directory of the matching `composer.lock` nearest to the root, minus the directory of any other active module's project inside it. Matching locks deeper than the nearest one are ignored; when several share the nearest depth, the first in path order wins. A user who works on a nested project sets `phplumb.root` to its folder. A module forced on without a matching lock reads from the root.
- **Symfony activation.** The setting `phplumb.frameworks.symfony` takes `auto`, `on` or `off`, like `phplumb.frameworks.drupal`. In `auto`, the module turns on when a `composer.lock` within the same depth that Drupal detection uses lists `symfony/framework-bundle`. Drupal projects do not require that package, so a plain Drupal project never activates it.
- **The extension sends Twig and Doctrine XML files** to the engine, in addition to PHP and YAML, so navigation works inside them without a Twig or XML language extension installed.
- **Synthetic fixtures only.** Repository fixtures for Symfony are small synthetic projects, like the Drupal fixtures.

## Delivery order

Each step is usable on its own and records its benchmark in `docs/benchmarks.md` for both reference projects:

1. **5a Built-in class members** from the stubs.
2. **5b Enums.**
3. **5c Element types.**
4. **6a Symfony module skeleton:** activation, the setting, project scoping for both modules, and the shared modules extracted from the Drupal module with no change in Drupal behavior.
5. **6b Routes**, in PHP and Twig.
6. **6c Templates**, in PHP and Twig.
7. **6d Messenger.**
8. **6e Parameters and YAML navigation.**
9. **6f Doctrine mapping.**
10. **6g Broken Symfony references.**

## Decisions made by the agent

The user decided the scope: Symfony navigation similar to Drupal's, for projects built on autowiring and attributes, with the core gaps fixed in the same work, and the reference project above. The user also decided that when several projects of the same framework share the workspace, the module reads only the one nearest to the root. The user has not explicitly confirmed these:

- **Delivery order** above, with the core steps first.
- **phpstorm-stubs as the source** of built-in members, shipped on disk.
- **Stubs for definitions only.** Built-in classes, functions, constants and members open their stubs; references, implementations, hierarchies and symbol search leave the stubs out, which keeps queries on names such as `count()` fast. Vendor polyfills of built-in names are not targets.
- **Coverage target** of 98% on the Symfony reference project.
- **Activation** on `symfony/framework-bundle`, and project scoping when both modules are active.
- **Shared internal modules** instead of a Symfony module that Drupal builds on.
- **Doctrine mapping inside the Symfony module**, active when the module is.
- **Non-goals:** events, environment variables, bundle service definitions, Twig expressions, the compiled container, and `$container->get()` typing.
- **Location.** The reference project's path stays out of the repository, as for the Drupal reference project.

## Left to the builder

- How the stubs are packaged and found at run time, whether they are indexed with the project or loaded once, and which stub extensions are included, as long as every name in `builtins.txt` with members has them.
- The exact list of built-in array functions whose callbacks receive element types, and how far generic template resolution goes, as long as the anti-goals hold.
- How the module reads routes from YAML route files and imported bundle routes, and which sources it can read with certainty.
- How Twig files are scanned for names, and how `when@env` blocks in configuration are treated.
- Setting names inside the extension other than `phplumb.frameworks.symfony`, crate and module layout, and test fixture design.
