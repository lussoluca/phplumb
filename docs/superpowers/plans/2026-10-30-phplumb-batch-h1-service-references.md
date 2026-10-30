# PHPlumb batch H1: service and parameter references implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lead more service and parameter references to their definitions: `@id` on an alias also to its service, `@!id` and `@>id`, `#[Autowire('@id')]` and `#[Autowire(null, 'id')]`, `%name%` after a lone `%`, and files loaded through `imports:`; read `config/packages/*.yml`, environment directories and imported files as Symfony loads them; add the parameter tests issue #40 left to #51.

**Architecture:** The configuration file list grows to what the kernel loads (`*.yaml` and `*.yml` in `config/packages` and its environment directories) plus the YAML files `imports:` names, read recursively once each; only the files the module still cannot read keep names unknown. Service references strip the `!`, `>` and `?` markers, and an alias's targets include its service's. Placeholders follow Symfony's `%%|%([^%\s]+)%`.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6d services and parameters, 6g broken references) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #49, #50, #51 and part of #48 (`config/packages/*.yml`, environment directories). Batch H2 takes the rest of #48: bundle services and parameters, XML and PHP configuration, parameters set from PHP code, `_instanceof` and `bind`.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** An import the module cannot read (a glob, a missing file, another format) keeps parameter names unknown, as today.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/49-service-references` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **An alias chain or an alias cycle** gives each service once and ends. Test in Task 1.
- **`'100%'` and `'%'` alone** give no parameter, and `'%%name%%'` stays an escape. Test in Task 2.
- **An import cycle** reads each file once. Test in Task 3.
- **`imports:` with `ignore_errors: true` on a missing file** keeps names certain, as Symfony ignores it. Test in Task 3.
- **`#[Autowire('@@x')]`** (an escaped `@`) is a value, not a service. Test in Task 2.

---

### Task 1: Aliases lead to their service (#49)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/config.rs`, `mod.rs`; tests in `tests/symfony.rs`.

**Interfaces:** `config::read_aliases(files, config_files) -> HashMap<String, String>`, each `id: '@other'` and `id: { alias: other }`; `Symfony::service_targets` gives the definition of `id`, then those of the services its alias chain leads to, each once.

- [ ] **Step 1: Write the failing tests.** `service_aliases_also_lead_to_their_service`, `alias_cycles_end`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(symfony): lead service aliases to their service` with `Closes: lussoluca/phplumb#49`.

### Task 2: More reference forms (#50, #51)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/config.rs`, `mod.rs`; tests in `tests/symfony.rs` and `config.rs`.

- [ ] **Step 1: Write the failing tests.** Unit tests on `parameter_at`: `'50% of %app.name%'` gives `app.name`; `'100%'`, `'%'` and `'%%a%%'` give nothing. `service_reference_markers_lead_to_the_service` (`@!id`, `@>id`, `@?id`); `autowire_service_forms_lead_to_the_service` (`#[Autowire('@id')]`, `#[Autowire(null, 'id')]`, and `#[Autowire('@@x')]` giving nothing); for #51, `other_template_arguments_give_nothing`, `aliased_and_qualified_autowire_attributes_lead_to_parameters`.
- [ ] **Step 2: Run them and watch them fail** (the #51 tests may pass; each must fail when its rule is removed).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read more service and parameter reference forms` with `Refs: lussoluca/phplumb#50` and `Closes: lussoluca/phplumb#51`.

### Task 3: Configuration files as the kernel loads them, and imports (#48, #50)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/config.rs`, `mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `config::config_files` lists `config/services.{yaml,yml}`, `config/services_<env>.{yaml,yml}`, `config/packages/*.{yaml,yml}` and `config/packages/<env>/*.{yaml,yml}`, then the YAML files their `imports:` name (relative to the importing file, read once each); `has_unread_configuration` no longer counts those, only PHP or XML configuration and imports the module cannot read.

- [ ] **Step 1: Write the failing tests.** `yml_and_environment_configuration_define_parameters`; `imported_files_define_parameters` (and the import's resource opens the file); `import_cycles_read_each_file_once`; `ignored_missing_imports_keep_names_certain`; `unreadable_imports_keep_names_unknown` (a glob).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read configuration files the kernel loads and their imports` with `Closes: lussoluca/phplumb#50` and `Refs: lussoluca/phplumb#48`.

### Task 4: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0.
- [ ] **Step 2:** Record `2026-10-30, batch H1 service references` in `docs/benchmarks.md`, format it, commit `docs: record batch h1 results`.
