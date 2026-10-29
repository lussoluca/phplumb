# PHPlumb batch E2: route references and bundle routes implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lead route names passed by name in PHP and Twig, and inside Twig string interpolations, to their routes; read the routes bundles define; and add the route tests issue #35 asks for.

**Architecture:** A named argument of a method call takes the position of the declared parameter with that name, so every framework reads `redirectToRoute(route: 'home')` like `redirectToRoute('home')`. The Twig reader finds string literals inside `#{...}` interpolations and accepts `name:` and `name =` arguments of `path()` and `url()`. The route reader resolves `@SomeBundle/...` resources through `config/bundles.php` and the bundle class's file, reads the bundle's route files like project ones, and gives the logout routes of the security firewalls a target in the security configuration.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (steps 6b routes and 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #33, #35 and #36. Open follow-ups read before planning: #90 (unread PHP route forms) stays open.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** A bundle route source the module cannot resolve or read keeps route names uncertain as before: `_` names for Symfony's bundles, every name for other bundles.
- **No lost results**, measured with the per-access dump and the route name comparison against `main`.
- **Branch** `fix/33-route-references` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A named argument the method does not declare,** or a variadic one, gives nothing. Test in Task 1.
- **A hash key `name:` inside `path('a', {name: 'b'})`** is not a route name. Test in Task 2.
- **An interpolation holding braces or quotes,** such as `"#{ {a: 'b'}|json_encode }"`, leaves the literals after it in the block readable. Test in Task 2.
- **A bundle that overrides `getPath()`,** or is not registered in `config/bundles.php`, keeps its routes uncertain. Test in Task 3.
- **A bundle import under a `name_prefix`** prefixes the bundle's route names. Test in Task 3.

---

### Task 1: Named arguments in PHP calls (#33)

**Files:** Modify `crates/phplumb-index/src/infer.rs`; fixture `tests/fixtures/symfony/src/Controller/LinksController.php`; tests in `tests/symfony.rs`.

**Interfaces:** `Ctx::argument_with` also finds the named argument covering the offset, at the position of the parameter of that name in the first call target that declares it; a name no target declares gives nothing.

- [ ] **Step 1: Write the failing tests.** Fixture lines `$this->redirectToRoute(route: 'home');`, `$this->urls->generate(name: 'about');` and `$this->redirectToRoute('x', status: 'home');` (`status` is not the route). Test `named_route_arguments_lead_to_their_routes`: the first two lead to `HomeController::index` and the `about` route; the third gives nothing.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** in `argument_with`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(infer): read named arguments at their parameter position` with `Refs: lussoluca/phplumb#33`.

### Task 2: Twig named arguments and interpolations (#33)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/twig.rs`; fixture `tests/fixtures/symfony/templates/broken.html.twig`; tests in `twig.rs` and `tests/symfony.rs`.

- [ ] **Step 1: Write the failing tests.** Unit tests in `reads_route_names`: `{{ path(name: 'home') }}`, `{{ url(name = 'home') }}`, `{{ path(parameters: {id: 1}, name: 'home') }}` and `{{ "#{path('home')}" }}` give `home`; `{{ path('a', {name: 'b'}) }}` on `'b'` and `{{ path(other: 'home') }}` give nothing; in `{{ "#{ {a: 'b'}|json_encode }" ~ path('home') }}` the last literal gives `home`. Fixture line `{{ "x#{path('nope_route')}" }}` and the diagnostics test expects `Undefined route 'nope_route'`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** literals inside `#{...}` of double-quoted strings (the innermost literal under the cursor wins) and named `name` arguments of `path` and `url`.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read twig route names passed by name or interpolated` with `Closes: lussoluca/phplumb#33`.

### Task 3: Bundle routes (#36)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/routes.rs`, `crates/phplumb-index/src/frameworks/symfony/config.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `Routes::read` takes the configuration files `config::config_files` lists; an `@Name/path` resource resolves to `<bundle dir>/path`, the bundle dir being the directory of the class registered in `config/bundles.php` whose short name is `Name` (its parent for an `AbstractBundle`), unknown when the class declares `getPath()` or is missing; `security.route_loader.logout` adds `_logout_<firewall>` for each firewall with `logout`, leading to its `logout` key, and keeps `_` names uncertain.

- [ ] **Step 1: Write the failing tests.** Temporary projects with `config/bundles.php`, a bundle class extending `Symfony\Component\HttpKernel\Bundle\Bundle` in `lib/` and its `Resources/config/routing/errors.php` closure adding `_preview_error`: `bundle_routes_lead_to_their_definitions` (`_preview_error` leads to the bundle file, `_missing` is flagged once every import is read); `abstract_bundles_read_routes_from_their_parent_directory`; `prefixed_bundle_imports_prefix_bundle_routes`; `unresolved_bundles_keep_routes_uncertain` (unregistered bundle and a bundle declaring `getPath()`); `logout_routes_lead_to_the_firewall` (`_logout_main` leads to `logout` in `config/packages/security.yaml`, and `_missing` stays silent).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the bundle resolution in `import()`, the logout routes, and pass the configuration files.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read the routes bundles define` with `Closes: lussoluca/phplumb#36`.

### Task 4: Route update and Twig guard tests (#35)

**Files:** `tests/symfony.rs`, fixture `tests/fixtures/symfony/templates/php.html.twig`.

- [ ] **Step 1: Write the tests.** On a copy of the fixture: `route_edits_update_route_targets` (rename the `home` route in `HomeController`, update, `home` gives nothing and the new name leads to `index`); `removed_controllers_lose_their_routes`; `route_file_edits_update_route_targets` (a key of `config/routes.yaml`); `added_xml_route_files_make_routes_uncertain`. Twig fixture holding `{{ path('home') }}` and `<?php new \App\Controller\CrudController(); \App\Nowhere::run(); ?>`: `twig_files_get_no_php_answers` asserts no hover, no definition, references or implementations on `CrudController`, the route still leading to its method, and no diagnostics.
- [ ] **Step 2: Run them.** Expected: they pass. Remove each guard and each update path in turn and watch its test fail; any test that passes without its behavior is rewritten.
- [ ] **Step 3: Commit** `test(symfony): cover route updates and twig guards` with `Closes: lussoluca/phplumb#35`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison, the route name comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0; bundle routes such as `_wdt` gain targets.
- [ ] **Step 2:** Record `2026-10-29, batch E2 route references` in `docs/benchmarks.md`, format it, commit `docs: record batch e2 results`.
