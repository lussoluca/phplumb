# PHPlumb for Zed

This extension runs the PHPlumb engine in Zed for PHP, YAML and Twig files. It gives definitions, implementations, references, call hierarchy, symbols, and the Drupal and Symfony features in the main README, with two gaps:

- **No type hierarchy:** Zed has no view for it.
- **No Doctrine XML:** Zed can only attach a server to every XML file, not to `*.orm.xml` alone, so the extension leaves XML out rather than start the engine for unrelated XML projects.

It runs on macOS (Apple silicon) and Linux (x64).

Zed starts the engine in every project where you open a PHP, YAML or Twig file, including projects without PHP, because an extension can't check for PHP files first. The cost there is small: on 137,000 files without PHP, indexing took under two seconds and the engine used about 70 MB of memory.

## Install

The extension is not in Zed's registry yet, so install it from this folder:

1. Install Zed's **PHP** extension, and the **Twig** extension for navigation in templates: Zed has no built-in support for either.
2. Install Rust with [rustup](https://rustup.rs/), then add Zed's WebAssembly target:

   ```sh
   rustup target add wasm32-wasip2
   ```

3. Restart Zed so it finds `cargo`.
4. Run `zed: install dev extension` from the command palette and pick the `editors/zed` folder.

On first start the extension downloads the latest PHPlumb release. Later starts check for a newer one, and keep using the installed release when GitHub can't be reached. Run `dev: open language server logs` to see the engine's log.

## Drupal files

Zed opens only `.php`, `.phtml` and `.phpt` files as PHP. VS Code switches Drupal's other PHP files to PHP on its own, but Zed can't, so map them in `settings.json`:

```json
{
  "file_types": {
    "PHP": ["module", "inc", "install", "theme", "profile", "engine"]
  }
}
```

Without it, navigation from those files, such as from a hook implementation, finds nothing.

## Settings

Set the engine's options under `lsp.phplumb` in Zed's `settings.json`. The keys of `initialization_options` match the VS Code settings without the `phplumb.` prefix:

```json
{
  "lsp": {
    "phplumb": {
      "initialization_options": {
        "exclude": [],
        "diagnostics": true,
        "frameworks": { "drupal": "auto", "symfony": "auto" }
      }
    }
  }
}
```

You rarely need `root`. Without it, the engine indexes the whole project and still finds Drupal and Symfony through their `composer.lock`, up to four folders deep. To index only one folder, set `root` per project in `.zed/settings.json`, for example `"root": "src/drupal"`. A global `root` that a project lacks makes the engine index the whole project and show a warning that names `phplumb.root`, the VS Code setting.

The extension keeps an index cache for each project in its own work folder, so reopening a project skips unchanged files. Nothing removes the cache of a project you move or delete: its folder under `cache` in the work folder can be deleted by hand. Set `cacheDir` in `initialization_options` to choose another folder.

To use another binary, such as a local debug build, set its path:

```json
{
  "lsp": {
    "phplumb": {
      "binary": { "path": "/path/to/phplumb" }
    }
  }
}
```

Without `binary.path`, the extension uses the downloaded release. It never takes `phplumb` from `PATH`, because the engine needs its PHP stubs in a `stubs` folder beside the binary.

## Other PHP servers

PHPlumb only navigates. Keep a PHP server such as Phpactor or Intelephense for completion and rename, and list it first so it stays the primary server:

```json
{
  "languages": {
    "PHP": { "language_servers": ["phpactor", "phplumb", "..."] }
  }
}
```

If both servers return the same definition twice, remove PHPlumb from PHP files with `"!phplumb"` in that list. YAML navigation keeps working.

## Unsaved changes

The engine reads files from disk. In a file with unsaved changes, positions can point to the wrong code. The VS Code extension hides results in that case, while Zed has no such hook, so turn on `autosave`, for example `{"autosave": {"after_delay": {"milliseconds": 500}}}`.
