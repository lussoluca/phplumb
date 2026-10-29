# PHPlumb batch E1: Symfony route definitions implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read the Symfony route definitions the "To fix" issues leave out: names from class constants, class prefixes set to `null`, aliases, localized routes, `name_prefix` imports, YAML files imported from outside `config/routes`, PHP route files, and route files as Symfony's kernel loads them.

**Architecture:** Attribute metadata gains `null` and class-constant values, and the summary keeps the string value of class constants, so route names written as `X::ROUTE` resolve. The route reader adds aliases and localized names, applies `name_prefix` to the attribute routes an import loads, reads imported YAML files wherever they are in the project, reads `add()` calls of PHP route files, and lists route files with the kernel's patterns. Each source it still cannot read keeps route diagnostics silent, as before.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (steps 6b routes and 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #29, #30, #31, #32 and #34. Batch E2 takes #33, #35 and #36.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** A route source the module cannot read keeps route diagnostics silent; a route the module now reads stays silent until every source is read.
- **No lost results**, measured with the per-access dump and the route name sweep against `main`.
- **Branch** `fix/29-route-definitions` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.
- Tests go in `tests/symfony.rs` (fixture `tests/fixtures/symfony`) and `tests/projects.rs` (temporary projects).

## Review focus

- **A constant whose value is not a string literal** (another constant, a concatenation) keeps route names uncertain. Test in Task 1.
- **A `name_prefix` import of a directory** prefixes only the routes of classes in that directory. Test in Task 3.
- **A PHP route file with an `import()` or a computed name** keeps route names uncertain. Test in Task 4.
- **A route file in a nested directory below `config/routes/<env>/`** is not read, as the kernel does not load it. Test in Task 5.

---

### Task 1: Route names from class constants and `name: null` (#29, #32)

**Files:** Modify `crates/phplumb-index/src/metadata.rs`, `crates/phplumb-index/src/summary.rs`, `crates/phplumb-index/src/frameworks/attributes.rs`, `crates/phplumb-index/src/frameworks/symfony/routes.rs`; tests and fixture.

**Interfaces:** Produces `MetaValue::Null` and `MetaValue::Constant { class: String, name: String }` (`self::X` and `static::X` keep `self`); `Member::value: Option<String>` (`#[serde(default)]`), a class constant's string literal value; `RouteName::Constant { class, name }`, which the Symfony reader resolves through the index (and `self` through the declaring class), uncertain when it does not resolve to a string.

- [ ] **Step 1: Write the failing tests.** Fixture: `LoginController` with `public const ROUTE = 'app_login'` and `#[Route('/login', name: self::ROUTE)]`, a second constant `OTHER = self::ROUTE` (not a literal) naming another route; a class-level `#[Route('/admin', name: null)]` with a method route `admin_home`. Tests: `routes_named_by_constants` (`app_login` leads to the method); `class_routes_named_null_have_no_prefix` (`admin_home` leads to its method); `constants_without_literal_values_keep_routes_uncertain` (in a temporary project, a missing route stays silent).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(symfony): read route names from class constants` with `Closes:` #29 and #32.

### Task 2: Aliases and localized routes (#31)

**Files:** Modify `crates/phplumb-index/src/frameworks/attributes.rs`, `crates/phplumb-index/src/frameworks/symfony/routes.rs`; tests and fixture.

- [ ] **Step 1: Write the failing tests.** Fixture: `#[Route('/a', name: 'aliased', alias: ['old_aliased'])]`; a YAML route `legacy_home: { alias: home }`; `#[Route(path: ['en' => '/about', 'it' => '/chi-siamo'], name: 'about')]`. Tests: `route_aliases_lead_to_their_route` (`old_aliased` to the method, `legacy_home` to the `home` definitions); `localized_routes_have_a_name_per_locale` (`about.en` and `about.it` to the method, `about` too); `localized_methods_get_no_class_alias` (`App\...::about` gives nothing). Aliases no longer make route names uncertain.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read route aliases and localized routes` with `Refs: lussoluca/phplumb#31`.

### Task 3: `name_prefix` imports and YAML files outside `config/routes` (#30, #31)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/routes.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** Temporary projects: `config/routes.yaml` importing `../src/Admin/` with `type: attribute` and `name_prefix: 'admin_'`, where `src/Admin/PanelController.php` declares `panel` and `src/Controller/HomeController.php` declares `home`: `admin_panel` leads to the method, `admin_home` gives nothing; `config/routes.yaml` importing `../modules/blog/routes.yaml` (a project file outside `config/routes`) whose `blog_index` route is read, and route names stay certain.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): apply name prefixes and read imported route files` with `Closes: lussoluca/phplumb#30`.

### Task 4: PHP route files (#31)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/routes.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** Temporary projects: `config/routes.php` with `$routes->add('php_home', '/')->controller([HomeController::class, 'index']);` gives `php_home` at its name and at the controller method, and keeps route names certain; a file with `$routes->import('...')` or `$routes->add($name, '/')` keeps them uncertain.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** reading `add()` calls with a literal name in `config/routes.php` and `config/routes/*.php`, with their `controller()` target when it is `[X::class, 'method']` or `X::class`.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read php route files` with `Closes: lussoluca/phplumb#31`.

### Task 5: Route files as the kernel loads them (#34)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/routes.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** Temporary projects: `config/routes/deep/nested/x.yaml` and `config/routes/y.yml` define nothing; `config/routes/dev/z.yaml` defines its route; a symlink loop below `config/routes` gives each target once; an update of `config/routes/A.YAML` reads it as at start.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the kernel's patterns (`config/routes.{yaml,php}`, `config/routes/*.{yaml,php}`, `config/routes/<env>/*.{yaml,php}`) and case-insensitive extensions in the update check.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): list route files as the kernel loads them` with `Closes: lussoluca/phplumb#34`.

### Task 6: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0, and its route names become certain if the class-constant routes were the only uncertain source.
- [ ] **Step 2:** Record `2026-10-29, batch E1 route definitions` in `docs/benchmarks.md`, format it, commit `docs: record batch e1 results`.
