# PHPlumb batch H2: service and parameter definitions implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lead service and parameter references to the definitions YAML does not hold: parameters set from PHP code, the project's PHP and XML configuration, and the services and parameters bundles define; and pin `_instanceof` and `bind` with tests.

**Architecture:** A `definitions` module reads parameters, services and aliases from PHP configurator files (`parameters()->set()`, `services()->set()`, `alias()`) and from XML service files (`<parameter key>`, `<service id>`, `<service alias>`). The project's PHP and XML configuration and each bundle's `Resources/config` and `config` directories are read with it, the bundles lazily on the first lookup after a read. These definitions add targets and known names; they never make a name certain, so PHP and XML configuration still keep parameter diagnostics silent.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6d services and parameters) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issue is the requirement.

**Issues:** lussoluca/phplumb #48 (rest). Open follow-ups read before planning: #93 stays open.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No false alarms.** Definitions read from PHP or XML only add targets and known names.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/48-service-definitions` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#48` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A `set()` call in a PHP configurator chain under `services()`** is a service, under `parameters()` a parameter, and one through a variable assigned from either follows it. Test in Task 2.
- **A `service('x')` reference inside `args()`** defines nothing. Test in Task 2.
- **A bundle whose directory the module cannot tell** adds nothing. Test in Task 3.
- **A computed name** (`set($name, ...)`) adds nothing. Test in Task 2.
- **Parameters set from PHP code outside the project** (vendor, `var`) add nothing. Test in Task 1.

---

### Task 1: Parameters set from PHP code (#48)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `Symfony::parameter_targets(&self, index, name) -> Vec<RouteTarget>`: the YAML definitions, then the literal names of project `setParameter()` and parameter bag `set()` calls.

- [ ] **Step 1: Write the failing test.** `parameters_set_from_php_lead_to_their_call`.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(symfony): lead parameters set from php to their call` with `Refs: lussoluca/phplumb#48`.

### Task 2: PHP and XML configuration (#48)

**Files:** Create `crates/phplumb-index/src/frameworks/symfony/definitions.rs`; modify `mod.rs`; tests in `definitions.rs` and `tests/projects.rs`.

**Interfaces:** `definitions::read_php(path, source) -> Definitions` and `definitions::read_xml(path, text) -> Definitions`, `Definitions { parameters, services: Vec<RouteTarget by name>, aliases: Vec<(String, String)> }`; the project's `config/**/*.php` and `config/**/*.xml` configuration files are read into the parameter, service and alias targets.

- [ ] **Step 1: Write the failing tests.** Unit tests for `read_php` (chains, variables, `alias()`, computed names, `service()` inside `args()`) and `read_xml`; `php_and_xml_configuration_define_targets` (a `%name%` and an `@id` lead there, and parameter diagnostics stay silent).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read php and xml service configuration` with `Refs: lussoluca/phplumb#48`.

### Task 3: Bundle services and parameters (#48)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`, `bundles.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `bundle_services_and_parameters_lead_to_their_definition` (a bundle's `Resources/config/routing.php` defines `router.default`, the alias `router` and `router.request_context.host`); `unresolved_bundles_define_nothing`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the lazy reading of every bundle's PHP and XML configuration.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read the services and parameters bundles define` with `Refs: lussoluca/phplumb#48`.

### Task 4: `_instanceof` and `bind` (#48)

**Files:** tests in `tests/projects.rs`.

- [ ] **Step 1: Write the tests.** `instanceof_and_bind_references_lead_to_their_definitions` (`@id` and `%name%` in `_instanceof` and `bind` lead to their definitions, and a missing `%name%` there is flagged).
- [ ] **Step 2: Run them.** A test that fails is a bug to fix with the test kept.
- [ ] **Step 3: Commit** `test(symfony): cover instanceof and bind references` with `Closes: lussoluca/phplumb#48`.

### Task 5: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; Symfony diagnostics stay at 0; `@router` and `%router.request_context.host%` lead to FrameworkBundle's configuration.
- [ ] **Step 2:** Record `2026-10-30, batch H2 service definitions` in `docs/benchmarks.md`, format it, commit `docs: record batch h2 results`.
