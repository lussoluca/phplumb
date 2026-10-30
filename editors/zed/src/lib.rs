//! The Zed extension: starts the PHPlumb engine, from the configured binary or the
//! latest release, and passes it the user's initialization options. Like the VS Code
//! extension, it never takes `phplumb` from PATH: the engine finds its PHP stubs
//! beside its binary, and a stray copy on PATH usually has none.
//!
//! The download code is adapted from the Symfony Language Tools extension for Zed;
//! see `THIRD_PARTY_NOTICES.md`.

use std::{env, fs};

use zed_extension_api::serde_json::{Map, Value, json};
use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, Architecture, LanguageServerId, Os, Result};

const LANGUAGE_SERVER_ID: &str = "phplumb";
const RELEASE_REPOSITORY: &str = "lussoluca/phplumb";
/// Prefix of the folders that each hold one downloaded release.
const INSTALLATION_PREFIX: &str = "phplumb-";
/// The binary's path inside a release package.
const BINARY_IN_PACKAGE: &str = "extension/bin/phplumb";
/// Suffix of a release folder still being unpacked.
const PARTIAL_SUFFIX: &str = ".partial";
/// Longest readable part of a cache folder name, well under the usual 255-byte
/// limit once the hash is added.
const CACHE_KEY_READABLE_MAX: usize = 200;
/// Folder of the index caches, one per project.
const CACHE_DIRECTORY: &str = "cache";

struct PhplumbExtension {
    cached_binary_path: Option<String>,
}

/// Where a release comes from and where it lands, relative to the extension's work
/// directory.
#[derive(Debug, PartialEq)]
struct ReleasePackage {
    asset_name: String,
    installation_directory: String,
}

impl zed::Extension for PhplumbExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        if language_server_id.as_ref() != LANGUAGE_SERVER_ID {
            return Err(format!("unknown language server: {language_server_id}"));
        }

        let binary = LspSettings::for_worktree(LANGUAGE_SERVER_ID, worktree)
            .ok()
            .and_then(|settings| settings.binary);
        let configured = binary
            .as_ref()
            .and_then(|binary| binary.path.clone())
            .filter(|path| !path.is_empty());
        let command = match configured {
            Some(command) => command,
            None => self.release_binary(language_server_id)?,
        };

        Ok(zed::Command {
            command,
            args: binary
                .as_ref()
                .and_then(|binary| binary.arguments.clone())
                .unwrap_or_else(|| vec!["serve".to_string()]),
            env: binary
                .and_then(|binary| binary.env)
                .unwrap_or_default()
                .into_iter()
                .collect(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<Value>> {
        let configured = LspSettings::for_worktree(LANGUAGE_SERVER_ID, worktree)
            .ok()
            .and_then(|settings| settings.initialization_options);

        initialization_options(configured, || {
            let directory = work_directory()?
                .join(CACHE_DIRECTORY)
                .join(cache_key(&worktree.root_path()));
            fs::create_dir_all(&directory)
                .map_err(|error| format!("failed to create the index cache folder: {error}"))?;
            Ok(directory.to_string_lossy().into_owned())
        })
        .map(Some)
    }
}

impl PhplumbExtension {
    /// The binary of the latest release, installed once per Zed session and reused
    /// for every worktree. When GitHub can't be reached or the install fails, the
    /// newest release already installed keeps the engine running.
    fn release_binary(&mut self, language_server_id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.cached_binary_path
            && is_file(path)
        {
            return Ok(path.clone());
        }

        let installation = match install_latest_release(language_server_id) {
            Ok(installation) => installation,
            Err(error) => {
                let Some(installed) = newest_installation(installation_folders(), |folder| {
                    is_file(&binary_in(folder))
                }) else {
                    zed::set_language_server_installation_status(
                        language_server_id,
                        &zed::LanguageServerInstallationStatus::Failed(error.clone()),
                    );
                    return Err(error);
                };
                // Zed writes an extension's stderr to its log.
                eprintln!("phplumb: {error}; starting the installed {installed}");
                installed
            }
        };
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::None,
        );

        // The engine finds its PHP stubs beside its own path, so it needs the full one.
        let binary_path = work_directory()?
            .join(binary_in(&installation))
            .to_string_lossy()
            .into_owned();
        self.cached_binary_path = Some(binary_path.clone());

        Ok(binary_path)
    }
}

/// Installs the latest release unless it is already installed, and returns the
/// folder that holds it.
fn install_latest_release(language_server_id: &LanguageServerId) -> Result<String> {
    zed::set_language_server_installation_status(
        language_server_id,
        &zed::LanguageServerInstallationStatus::CheckingForUpdate,
    );
    let release = zed::latest_github_release(
        RELEASE_REPOSITORY,
        zed::GithubReleaseOptions {
            require_assets: true,
            pre_release: false,
        },
    )?;
    let (os, architecture) = zed::current_platform();
    let package = release_package(&release.version, os, architecture)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == package.asset_name)
        .ok_or_else(|| format!("no release asset found matching {:?}", package.asset_name))?;

    let installation = package.installation_directory;
    if !is_file(&binary_in(&installation)) {
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );
        // The release is the VS Code package, a zip that bundles the binary. It is
        // unpacked beside the final folder and renamed once complete, so an install
        // cut short, even by Zed quitting, never looks complete.
        let partial = format!("{installation}{PARTIAL_SUFFIX}");
        fs::remove_dir_all(&partial).ok();
        zed::download_file(&asset.download_url, &partial, zed::DownloadedFileType::Zip)
            .and_then(|()| zed::make_file_executable(&binary_in(&partial)))
            .and_then(|()| replace_folder(&partial, &installation))
            .map_err(|error| {
                fs::remove_dir_all(&partial).ok();
                let folder = work_directory()
                    .map(|directory| directory.display().to_string())
                    .unwrap_or_else(|_| "the extension's work folder".into());
                format!("failed to install PHPlumb in {folder}: {error}")
            })?;
        // Only after a fresh install, so this Zed never removes the release it runs. A
        // second Zed instance sharing this folder, such as Zed Preview, can still lose
        // its older release here.
        remove_outdated_installations(&installation);
    }

    Ok(installation)
}

/// Moves `source` to `target`, removing a `target` left incomplete by an earlier run.
fn replace_folder(source: &str, target: &str) -> Result<()> {
    if fs::metadata(target).is_ok() {
        fs::remove_dir_all(target).map_err(|error| format!("cannot remove {target}: {error}"))?;
    }
    fs::rename(source, target).map_err(|error| format!("cannot move {source} to {target}: {error}"))
}

/// The binary's path inside a release folder.
fn binary_in(folder: &str) -> String {
    format!("{folder}/{BINARY_IN_PACKAGE}")
}

/// The folders in the work directory that hold a release, complete or not.
fn installation_folders() -> Vec<String> {
    let Ok(entries) = fs::read_dir(".") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(INSTALLATION_PREFIX))
        .collect()
}

/// The folder of the highest complete release among `folders`.
fn newest_installation(folders: Vec<String>, complete: impl Fn(&str) -> bool) -> Option<String> {
    folders
        .into_iter()
        .filter(|folder| !folder.ends_with(PARTIAL_SUFFIX) && complete(folder))
        .max_by_key(|folder| version_key(folder))
}

/// The numbers of the version in a folder named `phplumb-<version>-<platform>`, so
/// that 0.10.0 sorts after 0.9.0.
fn version_key(folder: &str) -> Vec<u64> {
    folder
        .trim_start_matches(INSTALLATION_PREFIX)
        .split('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Removes every release folder except `current`, including unfinished ones. Best
/// effort: a leftover folder never stops the engine.
fn remove_outdated_installations(current: &str) {
    for folder in installation_folders() {
        if folder != current {
            fs::remove_dir_all(folder).ok();
        }
    }
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// The user's initialization options, with a per-project `cacheDir` from
/// `default_cache_dir` when the user sets none. The engine writes its cache under a
/// fixed file name, so projects sharing one folder would overwrite each other.
fn initialization_options(
    configured: Option<Value>,
    default_cache_dir: impl FnOnce() -> Result<String>,
) -> Result<Value> {
    let mut options = match configured {
        Some(Value::Object(options)) => options,
        _ => Map::new(),
    };
    if !options.contains_key("cacheDir") {
        options.insert("cacheDir".to_string(), json!(default_cache_dir()?));
    }

    Ok(Value::Object(options))
}

/// A folder name for the cache of the project at `root`: the path, readable enough
/// to find and delete by hand, then a hash of it, since the readable part maps
/// paths such as `/a/b-c` and `/a-b/c` to the same name.
fn cache_key(root: &str) -> String {
    let readable: String = root
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();

    // A long path keeps its end, where the project's own name is. A leading '.'
    // would hide the folder.
    let skip = readable.len().saturating_sub(CACHE_KEY_READABLE_MAX);
    let readable = readable[skip..]
        .trim_start_matches(['-', '.'])
        .trim_end_matches('-');

    format!("{readable}-{:08x}", fnv1a(root))
}

/// The 32-bit FNV-1a hash, which stays the same across Rust versions, unlike the
/// standard library's hasher.
fn fnv1a(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

fn work_directory() -> Result<std::path::PathBuf> {
    env::current_dir()
        .map_err(|error| format!("failed to read the extension's work directory: {error}"))
}

fn platform_name(os: Os, architecture: Architecture) -> Result<&'static str> {
    match (os, architecture) {
        (Os::Mac, Architecture::Aarch64) => Ok("darwin-arm64"),
        (Os::Linux, Architecture::X8664) => Ok("linux-x64"),
        _ => {
            Err("PHPlumb releases no binary for this platform; set lsp.phplumb.binary.path".into())
        }
    }
}

fn release_package(version: &str, os: Os, architecture: Architecture) -> Result<ReleasePackage> {
    let platform = platform_name(os, architecture)?;
    let version = version.trim_start_matches('v');

    Ok(ReleasePackage {
        asset_name: format!("phplumb-{platform}-{version}.vsix"),
        installation_directory: format!("{INSTALLATION_PREFIX}{version}-{platform}"),
    })
}

zed::register_extension!(PhplumbExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_macos_arm64_release_package() {
        assert_eq!(
            ReleasePackage {
                asset_name: "phplumb-darwin-arm64-0.4.0.vsix".into(),
                installation_directory: "phplumb-0.4.0-darwin-arm64".into(),
            },
            release_package("v0.4.0", Os::Mac, Architecture::Aarch64).unwrap(),
        );
    }

    #[test]
    fn finds_the_binary_inside_a_release_folder() {
        assert_eq!(
            "phplumb-0.4.0-darwin-arm64/extension/bin/phplumb",
            binary_in("phplumb-0.4.0-darwin-arm64"),
        );
    }

    #[test]
    fn builds_linux_x64_release_package() {
        assert_eq!(
            "phplumb-linux-x64-0.4.0.vsix",
            release_package("0.4.0", Os::Linux, Architecture::X8664)
                .unwrap()
                .asset_name,
        );
    }

    #[test]
    fn rejects_platforms_without_a_release() {
        for (os, architecture) in [
            (Os::Mac, Architecture::X8664),
            (Os::Linux, Architecture::Aarch64),
            (Os::Windows, Architecture::X8664),
        ] {
            assert!(release_package("v0.4.0", os, architecture).is_err());
        }
    }

    #[test]
    fn adds_a_cache_dir_to_the_configured_options() {
        let options = initialization_options(Some(json!({"root": "src/drupal"})), || {
            Ok("/work/cache/project".into())
        });

        assert_eq!(
            json!({"root": "src/drupal", "cacheDir": "/work/cache/project"}),
            options.unwrap(),
        );
    }

    #[test]
    fn keeps_a_configured_cache_dir() {
        let options = initialization_options(Some(json!({"cacheDir": "/mine"})), || {
            panic!("the default cache folder must not be created")
        });

        assert_eq!(json!({"cacheDir": "/mine"}), options.unwrap());
    }

    #[test]
    fn adds_a_cache_dir_without_configured_options() {
        let options = initialization_options(None, || Ok("/work/cache/project".into()));

        assert_eq!(json!({"cacheDir": "/work/cache/project"}), options.unwrap());
    }

    #[test]
    fn names_the_cache_after_the_project_path() {
        assert_eq!(
            format!(
                "Users-me-Projects-my_site-{:08x}",
                fnv1a("/Users/me/Projects/my_site")
            ),
            cache_key("/Users/me/Projects/my_site"),
        );
    }

    #[test]
    fn gives_similar_project_paths_their_own_cache() {
        assert_ne!(cache_key("/a/b-c"), cache_key("/a-b/c"));
        assert_ne!(cache_key("/a/my site"), cache_key("/a/my-site"));
    }

    #[test]
    fn keeps_cache_names_of_deep_paths_short() {
        let root = format!("/{}my_site", "deep/".repeat(100));
        let key = cache_key(&root);

        assert!(key.len() <= CACHE_KEY_READABLE_MAX + 9);
        assert!(
            key.contains("-my_site-"),
            "the project's name stays readable"
        );
        assert_ne!(key, cache_key(&format!("{root}x")));
    }

    #[test]
    fn starts_a_cut_cache_name_without_a_dash() {
        // The readable part is 501 characters, so the cut falls on the second '/'.
        let root = format!("/{}/{}", "x".repeat(300), "y".repeat(199));

        assert!(cache_key(&root).starts_with('y'));
    }

    #[test]
    fn never_hides_a_cut_cache_name() {
        let root = format!("/{}/.config/{}", "a".repeat(100), "b".repeat(192));

        assert!(cache_key(&root).starts_with("config-"));
    }

    #[test]
    fn replaces_a_folder_left_by_an_earlier_run() {
        let base = env::temp_dir().join(format!("phplumb-replace-{}", std::process::id()));
        let (source, target) = (base.join("new"), base.join("old"));
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("binary"), "new").unwrap();
        fs::write(target.join("stale"), "old").unwrap();

        replace_folder(&source.to_string_lossy(), &target.to_string_lossy()).unwrap();

        assert!(target.join("binary").is_file());
        assert!(!target.join("stale").exists());
        assert!(!source.exists());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn picks_the_highest_complete_release() {
        let folders = vec![
            "phplumb-0.9.0-darwin-arm64".to_string(),
            "phplumb-0.10.0-darwin-arm64".to_string(),
            "phplumb-0.11.0-darwin-arm64.partial".to_string(),
            "phplumb-0.12.0-darwin-arm64".to_string(),
        ];
        let complete = |folder: &str| !folder.starts_with("phplumb-0.12.0");

        assert_eq!(
            Some("phplumb-0.10.0-darwin-arm64".to_string()),
            newest_installation(folders, complete),
        );
    }

    #[test]
    fn finds_no_release_when_none_is_complete() {
        let folders = vec!["phplumb-0.4.0-darwin-arm64".to_string()];

        assert_eq!(None, newest_installation(folders, |_| false));
    }

    #[test]
    fn hashes_with_fnv1a() {
        assert_eq!(0x811c_9dc5, fnv1a(""));
        assert_eq!(0xe40c_292c, fnv1a("a"));
    }
}
