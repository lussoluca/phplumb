# PHPlumb batch I2: Doctrine mappings beyond XML implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Navigate the class and property names of attribute association mappings, read YAML mapping files as XML ones are read, and type `getRepository('Alias:Entity')`.

**Architecture:** Attribute strings carry the class and property type they are declared on and the attribute's arguments, so the Symfony module resolves `targetEntity`, `inversedBy` and `mappedBy` like the XML reader. YAML mapping files (`*.orm.yml`, `*.orm.yaml`) get the same names and repositories as XML ones. Doctrine's mapping aliases from `doctrine.orm.mappings` turn `Alias:Entity` into a class.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6f Doctrine) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issue is the requirement.

**Issues:** lussoluca/phplumb #52. Strings inside `@ORM\...` annotations get a follow-up issue.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No wrong jumps.** A target entity the module cannot tell gives nothing for `inversedBy` and `mappedBy`.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/52-doctrine-mappings` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#52` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **`mappedBy` on a collection property without `targetEntity`** gives nothing. Test in Task 1.
- **A short `targetEntity` string** takes the entity's namespace. Test in Task 1.
- **A YAML mapping file outside `config`** is read like one inside. Test in Task 2.
- **An alias Doctrine does not know** keeps `getRepository()` with its declared type. Test in Task 3.
- **Attributes of other classes with the same argument names** give nothing. Test in Task 1.

---

### Task 1: Attribute association names

**Files:** Modify `crates/phplumb-index/src/frameworks/mod.rs`, `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `AttributeString::holder: Option<String>`, `holder_type: Option<TypeExpr>` and `arguments: Vec<(Option<String>, MetaValue)>`.

- [ ] **Step 1: Write the failing tests.** `association_attributes_lead_to_classes_and_properties` (`targetEntity` string, `inversedBy` through `targetEntity` and through the property type, `mappedBy`); the review focus cases.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p phplumb-index && cargo clippy -p phplumb-index --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(doctrine): navigate association attributes` with `Refs: lussoluca/phplumb#52`.

### Task 2: YAML mapping files

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/doctrine.rs`, `mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `yaml_mappings_navigate_and_type_repositories` (entity key, `repositoryClass`, field and association keys, `targetEntity`, `inversedBy`; `getRepository()` typed).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(doctrine): read yaml mapping files` with `Refs: lussoluca/phplumb#52`.

### Task 3: Entity aliases

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/doctrine.rs`, `mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `aliased_entities_type_their_repository` (`doctrine.orm.mappings.App: { prefix: App\Entity, alias: Shop }` gives `getRepository('Shop:Post')` its repository; a mapping without `alias` uses its name; an unknown alias keeps the declared type).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the `phplumb-index` suite and clippy.
- [ ] **Step 5: Commit** `fix(doctrine): type repositories of aliased entities` with `Closes: lussoluca/phplumb#52`.

### Task 4: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases, and the full workspace suite. Expected: no lost or changed target.
- [ ] **Step 2:** Record `2026-10-30, batch I2 Doctrine mappings` in `docs/benchmarks.md`, format it, commit `docs: record batch i2 results`.
