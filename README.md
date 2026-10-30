<p align="center">
  <img src="docs/images/logo.svg" width="128" height="128" alt="PHPlumb logo, a letter P made of pipes">
</p>

<h1 align="center">PHPlumb</h1>

<p align="center"><strong>Fast code navigation in VS Code for reviewing PHP that AI agents write.</strong></p>

Jump from a call to its definition, from an interface to its implementations, and from a method to every place that calls it, in a project of 40,000 PHP files, without waiting.

![PHPlumb on Drupal core: Cmd+click an entity type id, find all references of load(), follow a service call and open a service definition](docs/images/demo.gif)

## Why

AI agents write a lot of PHP, fast. Reviewing it means reading: following calls across classes to check that the code is correct and does what you asked.

The PHP extensions for VS Code make that slow. Indexing a large project takes a long time, the free tier of Intelephense has no "find implementations", and on Drupal code the type of an object is lost as soon as it leaves the service container, so navigation stops there.

PHPlumb is built for that review loop. It has its own engine, written in Rust, and does one thing: navigation.

## Features

- **Go to definition** for classes, interfaces, traits, enums, functions, methods, properties, constants and enum cases, through type inference and PHPDoc types. Names in docblocks work too: classes in PHPDoc types, members and functions in `@see`, and annotation names.
- **Find implementations** of an interface, an abstract class or a method, transitively.
- **Find all references**, including calls made through the interface or parent method that a method implements.
- **Call hierarchy** with incoming and outgoing calls, and **type hierarchy** with supertypes and subtypes.
- **Hover** with the resolved type, the signature and the docblock.
- **Symbol search** across the workspace (Cmd+T) and the outline of the current file.
- **Broken reference diagnostics.** Classes, functions, methods and constants that exist nowhere are flagged in project code, so code an agent invented stands out. A warning on working code is a bug. Members of PHP's built-in classes are never flagged, since the stubs describe one PHP version, except on `UnitEnum` and `BackedEnum`, whose members PHP fixes.

Agents edit files outside the editor, often many at once. PHPlumb follows changes on disk, including a branch switch, without you opening the files. The index is cached, so reopening a project skips files that did not change.

## Drupal

PHPlumb starts with Drupal, because its code calls almost everything through interfaces and services. What works today:

- `.module`, `.inc`, `.install`, `.theme`, `.profile` and `.engine` files are indexed as PHP, and VS Code opens them as PHP.
- Inside `vendor`, only the code that Composer's autoloader reaches is indexed, so package tests and leftover copies never show up as targets.
- Broken reference diagnostics skip core and contrib, like `vendor`, and are tested against them for false alarms.
- **Annotations.** `@Block(...)`, `@Translation(...)` and other plugin annotations lead to their annotation class, imported or not, as plugin discovery resolves them.
- **Services.** `\Drupal::service('id')` and `$container->get('id')` are typed from every `*.services.yml`, so navigation continues on the next call. Cmd+click on a service id opens its definition.
- **Entities.** `getStorage('node')` is typed as the storage class of that entity type, and `->load()` as the entity class, from entity types declared by attributes or annotations. Cmd+click on an entity type id opens its declaration.
- **Hooks.** Cmd+click on a hook implementation, a `#[Hook]` method or a procedural `mymodule_form_alter()`, opens the places that invoke the hook and its `hook_*` documentation. Cmd+click on the hook name in `invokeAll('cron')` or `alter('form')` lists every implementation. Hooks invoked under computed names, such as `hook_form_FORM_ID_alter` or `hook_ENTITY_TYPE_presave`, lead to the core code that computes them.
- **Plugins.** Cmd+click on a plugin id opens its class: `'#type' => 'textfield'`, `BaseFieldDefinition::create('string')`, widget and formatter types in `setDisplayOptions()`, `addConstraint('Name')` and `createInstance('id')` on a plugin manager. Go to implementations on a plugin manager lists all its plugins. Plugins are found by attribute or annotation, the way Drupal discovers them.
- **Routes and YAML.** Cmd+click on the route name in `Url::fromRoute('name')`, `Link::createFromRoute()`, `setRedirect()` or `redirect()` opens the route in `*.routing.yml`. In services, routing and links YAML files, Cmd+click on a class, a `Class::method`, a `service:method` controller, an `@service` or a route name opens its definition.
- **Broken Drupal references.** A service id, route name or plugin id written in PHP that is defined nowhere on disk, test modules included, is reported: `\Drupal::service('x')`, `Url::fromRoute('x')`, `'#type' => 'x'`, field types, widgets and formatters. Ids that code builds at run time, and lookups that tolerate a missing id, stay silent. Nothing is reported while `phplumb.exclude` is set, since ids defined in excluded files are unknown.

## Symfony

PHPlumb turns on its Symfony support when the project's `composer.lock` requires `symfony/framework-bundle`. What works today:

- **Routes.** Cmd+click on a route name in `redirectToRoute()`, `generateUrl()`, `UrlGeneratorInterface::generate()`, or `path()` and `url()` in a Twig template, opens the controller method of a `#[Route]` attribute, or the route in `config/routes.yaml` and its controller. Route names follow Symfony's rules: class name prefixes, generated default names and `Class::method` aliases.
- **Templates.** Cmd+click on a template name in `render()`, `renderView()`, `#[Template]`, or in Twig `extends`, `include`, `embed`, `import`, `from`, `use`, `include()` and `source()`, opens the Twig file. Namespaces such as `@Blog` come from `twig.paths` in `config/packages/twig.yaml`, and plain names resolve against those paths without a namespace, then `templates/`, then TwigBridge's form themes such as `form_div_layout.html.twig`.
- **Messenger.** Cmd+click on `dispatch` in `$bus->dispatch(new Message())` lists every handler of the message, and Cmd+click on a handler lists every place that dispatches its message. Handlers are classes or methods with `#[AsMessageHandler]`, and a handler of a parent class or interface of the message handles it too.
- **Parameters.** Cmd+click on a parameter name in `#[Autowire(param: 'x')]`, `#[Autowire('%x%')]`, `getParameter('x')` or a `%x%` in YAML opens its definitions under `parameters:`, including those inside `when@env` blocks.
- **YAML configuration.** In `config/services*.yaml` and `config/packages/*.yaml`, Cmd+click on a class name opens the class, and on `@id` (or `#[Autowire(service: 'id')]`) opens the service definition, or the class when the id is a class name with no definition.
- **Doctrine.** In `*.orm.xml` mapping files, Cmd+click on an entity, embeddable, target, repository or enum class opens the class, and on a field, association or id name opens the property. `getRepository(X::class)` is typed as the repository class the mapping or `#[ORM\Entity(repositoryClass:)]` declares.
- **Broken references.** Route, template and parameter names that are defined nowhere are flagged in PHP, Twig templates and YAML configuration. A name stays silent whenever a source that could define it cannot be read: a route import from an unknown bundle, a route named by a class constant, an unconfigured Twig namespace, PHP configuration, or a parameter set by PHP code with a computed name. Parameters whose first segment no project parameter shares, such as `router.request_context.host`, belong to bundles and stay silent too.

## Speed

Measured on a Drupal 11 project with 39,417 PHP files, on an Apple M3 Pro.

| Task                                                    | Time          |
| ------------------------------------------------------- | ------------- |
| Index the whole project from scratch                    | 2 to 4 s      |
| Start again from the cache                              | 1.7 s         |
| Find 1,033 references of `EntityStorageInterface::load` | 100 to 135 ms |
| Search symbols across the workspace                     | about 30 ms   |

Peak memory stays under 500 MB. See [the benchmark history](docs/benchmarks.md) for every run.

## Install

PHPlumb runs on macOS (Apple silicon) and Linux (x64), in VS Code 1.140 or later.

1. Download the `.vsix` for your platform from the [releases page](https://github.com/lussoluca/phplumb/releases).
2. Install it:

   ```sh
   code --install-extension phplumb-darwin-arm64-0.6.0.vsix
   ```

3. Open a PHP project. Indexing progress appears in the status bar.

**Disable Intelephense and other PHP navigation extensions** in workspaces where you use PHPlumb, or you will see every result twice.

For Zed, see [the Zed extension](editors/zed/README.md).

## Settings

| Setting                       | Default  | Description                                                                                                                                                                        |
| ----------------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `phplumb.root`                | `""`     | Folder of the PHP project, relative to the workspace folder, such as `src/drupal`. Only this folder is indexed. Empty uses the workspace folder.                                   |
| `phplumb.exclude`             | `[]`     | Glob patterns, relative to the project folder (`phplumb.root`), of paths not to index.                                                                                             |
| `phplumb.diagnostics.enabled` | `true`   | Reports names that do not exist, in project code. Dependencies, such as `vendor`, are skipped.                                                                                     |
| `phplumb.frameworks.drupal`   | `"auto"` | Drupal support: `auto` turns it on when a `composer.lock` in the project folder or up to three directories below it requires `drupal/core`; `on` or `off` forces it.               |
| `phplumb.frameworks.symfony`  | `"auto"` | Symfony support: `auto` turns it on when a `composer.lock` in the project folder or up to three directories below it requires `symfony/framework-bundle`; `on` or `off` forces it. |
| `phplumb.server.path`         | `""`     | Path of the `phplumb` binary. Empty uses the bundled binary.                                                                                                                       |
| `phplumb.trace.server`        | `"off"`  | Traces the communication between VS Code and the engine: `off`, `messages` or `verbose`.                                                                                           |

Each framework module reads and answers only for the folder of its `composer.lock`. When one project sits inside another, such as a Drupal codebase inside a Symfony application, the outer module leaves the inner folder to the inner one.

The **PHPlumb: Restart Server** command restarts the engine. The **PHPlumb** output channel shows its log.

## What it does not do

PHPlumb is for reading code. It has no autocomplete, signature help, rename or other refactorings, formatting, code actions or debugging.

## Status

PHPlumb is young (0.6.0). Core PHP navigation, diagnostics and the Drupal and Symfony features above are complete.

## Development

PHPlumb has two parts: an engine in Rust (`crates/`) that speaks the Language Server Protocol, and thin extensions for VS Code (`editors/vscode`) and Zed (`editors/zed`) that start it. See the [development guide](docs/development.md) to build, test and package it.

## License

[MIT](LICENSE). The list of PHP built-in names comes from [JetBrains phpstorm-stubs](https://github.com/JetBrains/phpstorm-stubs), under the Apache License 2.0; see [its notice](crates/phplumb-index/data/README.md).

The website in `site/` is built on a Tailwind Plus template and is not covered by the MIT license; see [its README](site/README.md).
