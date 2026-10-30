# PHPlumb batch G2: Messenger handlers, schedules and buses implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Follow the Messenger flows batch G1 left: handlers under a symlinked root, wildcard `handles`, messages the Scheduler sends, handlers configured in YAML with the `messenger.message_handler` tag, and handlers limited to one bus.

**Architecture:** Handler lookups by position compare paths as the index stores them. A handler's handled classes may be `*` or a namespace wildcard, matched as `HandlersLocator` lists types. `RecurringMessage::every()`, `cron()` and `trigger()` join `dispatch()` with the message at position 1. Service definitions tagged `messenger.message_handler`, directly or through `_instanceof`, add handlers. A handler's `bus` limits it to dispatches whose bus the module can tell from the receiver's autowiring name.

**Tech Stack:** existing workspace; no new dependency.

**Spec:** `docs/superpowers/specs/2026-10-28-phplumb-symfony-design.md` (step 6e Messenger) and `docs/superpowers/specs/2026-10-26-php-navigator-design.md` (anti-goals). The issues are the requirements.

**Issues:** lussoluca/phplumb #41 (rest), #42 and #45. Open follow-up #92 (trait conflict resolution) stays open.

## Global constraints

- Every earlier plan's global constraint still holds.
- **No wrong jumps.** A dispatch's bus counts only when the module can tell it: a promoted constructor property or a parameter named like a configured bus's autowiring alias, or the default bus for any other name; a parameter with an attribute, or Messenger configuration the module cannot read, leaves the bus unknown, and then every handler stays listed.
- **No lost results**, measured with the per-access dump and both benchmarks against `main`.
- **Branch** `fix/41-messenger-routing` from `main`. Conventional commits with `Closes:` or `Refs: lussoluca/phplumb#N` and `Assisted-by: claude-code/claude-opus-5-5`.

## Review focus

- **A namespace wildcard `App\Message\*`** matches classes in that namespace and below, never a class whose name only starts the same way (`App\MessageX`). Test in Task 2.
- **A tagged service whose class the index does not know,** or with a `method` it lacks, adds no handler. Test in Task 4.
- **A bus property assigned from a differently named constructor parameter** leaves the bus unknown. Test in Task 5.
- **A handler without `bus`** stays listed for every bus. Test in Task 5.
- **`RecurringMessage` with its message not at position 1** (named arguments) still reaches its handlers. Test in Task 3.

---

### Task 1: Handlers under a symlinked root (#45)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing test.** `handlers_under_a_symlinked_root_list_their_dispatches`: a project reached through a symlink; definition on a handler method, with the path given through the link, lists its dispatch.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement** comparing the path the index stores (`filter().under_root`) in `declaration_targets`.
- [ ] **Step 4: Run every suite.** Run: `cargo test --workspace && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(symfony): find handlers under a symlinked root` with `Closes: lussoluca/phplumb#45`.

### Task 2: Wildcard handlers (#41)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/messenger.rs`; tests in `tests/projects.rs`.

- [ ] **Step 1: Write the failing tests.** `wildcard_handlers_handle_every_message` (`handles: '*'`); `namespace_wildcard_handlers_handle_their_namespace` (`App\Message\*` handles `App\Message\Sub\X`, not `App\MessageX`); both list their dispatches back.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement** the match.
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read wildcard message handlers` with `Refs: lussoluca/phplumb#41`.

### Task 3: Scheduler messages (#41)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** the message calls become `(owner, method, position)`: `MessageBusInterface::dispatch` and `HandleTrait::handle` at 0, `Symfony\Component\Scheduler\RecurringMessage::every`, `cron` and `trigger` at 1.

- [ ] **Step 1: Write the failing tests.** `recurring_messages_reach_their_handlers` (definition on `every` lists the handlers, and the handler lists the call, also with `message:` given by name).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): follow messages the scheduler sends` with `Refs: lussoluca/phplumb#41`.

### Task 4: Handlers tagged in YAML (#41)

**Files:** Modify `crates/phplumb-index/src/frameworks/symfony/config.rs`, `messenger.rs`, `mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `config::tagged_services(files, config_files, tag) -> Vec<(String class, Vec<(String, String)> attributes)>`, each service defined with the tag (by name or `{ name: tag, ... }`), and each `_instanceof` interface with it; `Handlers::read` adds a handler for each, with the tag's `handles` and `method` (`__invoke` by default).

- [ ] **Step 1: Write the failing tests.** `tagged_handlers_handle_their_messages` (string and map forms, `method` and `handles`); `instanceof_tagged_handlers_handle_their_messages`; `tagged_services_the_index_lacks_add_nothing`.
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): read message handlers tagged in yaml` with `Closes: lussoluca/phplumb#41`.

### Task 5: Handlers of one bus (#42)

**Files:** Modify `crates/phplumb-index/src/infer.rs`, `crates/phplumb-index/src/frameworks/mod.rs`, `symfony/messenger.rs`, `symfony/config.rs`, `symfony/mod.rs`; tests in `tests/projects.rs`.

**Interfaces:** `FoundCall::receiver_name` and `CallAt::receiver_name: Option<Receiver>` (`This(property)` or `Variable(name)`); `config::buses(files, config_files) -> Option<(Vec<String>, String)>`, the configured buses and the default, none when unread; the handler's `bus` option; a dispatch's bus as the constraint above states.

- [ ] **Step 1: Write the failing tests.** Two buses `command.bus` (default) and `query.bus`; `bus_handlers_list_only_dispatches_of_their_bus` (`$this->queryBus->dispatch()` through a promoted property lists the `query.bus` handler and not the `command.bus` one, and the other way); `handlers_without_bus_stay_listed`; `buses_the_module_cannot_tell_keep_every_handler` (assigned property, attributed parameter).
- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run every suite.**
- [ ] **Step 5: Commit** `fix(symfony): list only the handlers of a dispatch's bus` with `Closes: lussoluca/phplumb#42`.

### Task 6: Benchmarks

- [ ] **Step 1:** Build `main` and this branch; run the per-access dump comparison and both benchmarks with diagnostics and cases. Expected: no lost or changed target; the reference's `RecurringMessage::every(...)` reaches its handler.
- [ ] **Step 2:** Record `2026-10-30, batch G2 Messenger routing` in `docs/benchmarks.md`, format it, commit `docs: record batch g2 results`.
