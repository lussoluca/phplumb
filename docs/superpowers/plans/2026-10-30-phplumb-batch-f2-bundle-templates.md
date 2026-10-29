# PHPlumb batch F2: bundle templates and template names in YAML implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open templates of bundle namespaces such as `@Twig` or `@WebProfiler`, with the project's overrides in `templates/bundles` first as TwigBundle adds them, and open template names written in YAML configuration such as `twig.form_themes`.

**Architecture:** The bundle resolution batch E2 wrote for routes moves to a `bundles` module that lists every bundle of `config/bundles.php` with its directory. The template reader adds, for each bundle, the namespace TwigBundle derives from its name, with the override directory and then the bundle's views, and the `!Name` namespace with the views alone. YAML configuration values ending in `.twig` open their template.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6c templates, 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #37 and #38. Open follow-ups read before planning: #91 (a bundle that sets its own path) stays open and keeps its namespace unknown.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** A bundle whose directory the module cannot tell, or whose override directory depends on configuration it does not read, keeps its namespace unknown; bundle templates themselves get no diagnostics.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/37-bundle-templates` from `main`. Conventional commits with `Closes: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **`@!Name/...`** skips the override and opens the bundle's own template. Test in Task 2.
- **A bundle class the module cannot resolve, or with its own `getPath()`,** keeps `@Name/...` unknown, never missing. Test in Task 2.
- **An environment `default_path`** keeps bundle namespaces unknown, since their overrides move with it. Test in Task 2.
- **A template inside a bundle's views** gets no diagnostics. Test in Task 2.
- **A YAML value ending in `.twig` that starts with `@`** opens the template, not a service. Test in Task 3.

---

### Task 1: The bundles module

**Files:** Create `crates/phplumb-index/src/frameworks/symfony/bundles.rs`; modify `routes.rs`, `mod.rs`.

**Interfaces:** Produces `bundles::bundles(index: &Index, config: &Path) -> Vec<(String, Option<PathBuf>)>`, each bundle of `config/bundles.php` in order with its short class name and its directory (none when the module cannot tell), and `bundles::bundle_dir(index, config, name) -> Option<PathBuf>`; `routes.rs` uses the latter. No behavior changes: the route tests of batch E2 pass unchanged.

- [ ] **Step 1:** Move the code; run `cargo test --workspace && cargo clippy --all-targets -- -D warnings`. Expected: all pass.
- [ ] **Step 2: Commit** `refactor(symfony): move bundle resolution to its own module` with `Refs: lussoluca/phplumb#37`.

### Task 2: Bundle template namespaces (#37)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/templates.rs`, `mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `Templates::read(index, project, config_files)`; for each bundle, after the configured paths and before `default_path`: namespace `Name` (the bundle name without its `Bundle` suffix) with `<default_path>/bundles/<BundleName>` when it exists, then `<dir>/Resources/views` or else `<dir>/templates` when one exists; namespace `!Name` with the views alone; an unknown directory, unreadable `default_path` or environment `default_path` gives the namespace an unreadable entry. `holds()` leaves bundle views out.

- [ ] **Step 1: Write the failing tests.** On `bundled_symfony` with views and the bridge: `bundle_templates_open_from_their_views` (`@Framework/x.html.twig` in `Resources/views`, `@Acme/card.html.twig` in an `AbstractBundle`'s `templates`, `@Acme/gone.html.twig` missing); `bundle_overrides_come_first` (`templates/bundles/AcmeBundle/card.html.twig` for `@Acme`, the bundle's own for `@!Acme`); `unresolved_bundle_namespaces_stay_unknown` (own `getPath()`, environment `default_path`); `bundle_templates_get_no_diagnostics`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): open templates of bundle namespaces` with `Closes: lussoluca/phplumb#37`.

### Task 3: Template names in YAML (#38)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`; fixture `config/packages/twig.yaml`; tests in `tests/symfony.rs`.

- [ ] **Step 1: Write the failing test.** `template_names_in_yaml_open_their_file`: `form_themes: ['page.html.twig', '@Blog/list.html.twig']` in the fixture's `twig.yaml` open their files; a value naming no template opens nothing.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement** in `yaml_targets`: a value ending in `.twig` opens its template, before the service reference rule. YAML template names get no diagnostics.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): open template names in yaml configuration` with `Closes: lussoluca/phplumb#38`.

### Task 4: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0; `form/theme.html.twig` in the reference's `twig.yaml` opens its file.
- [ ] **Step 2:** Record `2026-10-30, batch F2 bundle templates` in `docs/benchmarks.md`, format it, commit `docs: record batch f2 results`.
