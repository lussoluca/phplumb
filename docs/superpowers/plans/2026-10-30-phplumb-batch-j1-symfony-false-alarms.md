# PHPlumb batch J1: Symfony false alarms implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep Symfony diagnostics silent when a source the module does not see may define the name: kernel overrides, `framework.router.resource`, custom Twig loaders, parameters bundle extensions set, and parameters set through untyped variables; pin a namespace both `twig.paths` and a bundle register.

**Architecture:** A project kernel declaring `getConfigDir()`, `loadRoutes()` or `getKernelParameters()` leaves the kinds of names it can change unknown, as `configureRoutes()` does for routes. A `framework.router.resource` other than `kernel::loadRoutes` leaves routes unknown. A project class implementing Twig's `LoaderInterface`, or a service tagged `twig.loader`, leaves templates unknown. Each bundle's `DependencyInjection` code is read for the parameters its `setParameter()` calls set: literal names, literal suffixes after a computed start, and otherwise every name unknown. Every project file that calls `setParameter()` is read, and an untyped receiver's literal name counts.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #59 and #61. Batch J2 takes #57 and #58. Open follow-ups read before planning: #93, #94 and #97 stay open.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** A source that may define a name keeps that kind of name unchecked.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/59-symfony-false-alarms` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A class that is not a kernel** declaring `getConfigDir()` changes nothing. Test in Task 1.
- **`framework.router.resource` inside a `when@` block** leaves routes unknown too. Test in Task 1.
- **A Doctrine `$query->setParameter('id', $v)` with a known receiver type** defines no parameter. Test in Task 4.
- **An untyped computed `setParameter($name, $v)`** in a file that mentions no container class keeps other parameters checked. Test in Task 4.
- **A bundle whose directory the module cannot tell** adds no extension parameters and silences nothing more than today. Test in Task 3.

---

### Task 1: Kernel overrides and the router resource (#59)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`, `routes.rs`, `templates.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `kernel_overrides_keep_names_unknown`: a kernel (extends `Symfony\Component\HttpKernel\Kernel`) declaring `getConfigDir()` leaves an undefined route, template and parameter unflagged; one declaring `loadRoutes()` leaves only routes unflagged; one declaring `getKernelParameters()` leaves only parameters unflagged; a plain class declaring `getConfigDir()` keeps all three flagged. `router_resource_keeps_routes_unknown`: `framework.router.resource` set to another file, at top level and inside `when@dev`, leaves an undefined route unflagged; set to `kernel::loadRoutes` keeps it flagged.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): silence names kernel overrides may define` with `Refs: lussoluca/phplumb#59`.

### Task 2: Custom Twig loaders and shared namespaces (#59)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/templates.rs`, `mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the tests.** `custom_twig_loaders_keep_templates_unknown`: a project class implementing `Twig\Loader\LoaderInterface`, and a YAML service tagged `twig.loader` (string and map forms), each leave an undefined template unflagged. `namespaces_from_paths_and_bundles_read_both`: `@Acme/a.html.twig` in the `twig.paths` directory and `@Acme/b.html.twig` in the bundle's views both open, and `@Acme/c.html.twig` in neither is flagged.
- [ ] **Step 2: Run them.** Expected: the loader test fails; the namespace test passes and fails when either directory is dropped.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): silence templates custom twig loaders may load` with `Refs: lussoluca/phplumb#59`.

### Task 3: Parameters bundle extensions set (#59)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/definitions.rs`, `mod.rs`; tests in `definitions.rs` and `tests/projects.rs`.

**Interfaces:** `definitions::extension_parameters(files) -> ExtensionParameters { names: HashSet<String>, suffixes: Vec<String>, any: bool }`, read from each bundle directory's `DependencyInjection` PHP files and its bundle class; `parameter_missing` is false for a name among `names`, ending with one of `suffixes`, or when `any`.

- [ ] **Step 1: Write the failing tests.** Unit tests on `extension_parameters`: `setParameter('router.resource', ...)` gives a name; `setParameter($busId.'.middleware', ...)` gives the suffix `.middleware`; `setParameter($name, ...)` gives `any`. `bundle_extension_parameters_stay_silent`: with a project parameter `router.request_context.host`, `%router.resource%` and `%command.bus.middleware%` (project parameter `command.x`) are not flagged, and `%router.missing%` is.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement**, cached with the bundle definitions.
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): read the parameters bundle extensions set` with `Closes: lussoluca/phplumb#59`.

### Task 4: Parameters set through untyped variables (#61)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `parameters_set_through_untyped_variables_are_known`: a project file mentioning no container class with `function ($c) { $c->setParameter('app.extra', 1); }` keeps `%app.extra%` unflagged and leads there; in the same project a `$query->setParameter('app.other', 1)` on a typed Doctrine query defines nothing, and an untyped `$c->setParameter($name, 1)` keeps `%app.missing%` flagged.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement**: read every project file with a `setParameter` member; the container-class filter stays for `getParameterBag`.
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(symfony): read parameters set through untyped variables` with `Closes: lussoluca/phplumb#61`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0.
- [ ] **Step 2:** Record `2026-10-30, batch J1 Symfony false alarms` in `docs/benchmarks.md`, format it, commit `docs: record batch j1 results`.
