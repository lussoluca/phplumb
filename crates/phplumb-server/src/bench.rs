//! `phplumb bench`: indexes a project, measures it and checks a case file.
//!
//! Case positions are `relative/path:line:column`, 1-based, with columns in UTF-16
//! units, pointing at the start of a name. An empty `expect` means "nothing".
//! References cases exclude the declaration.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use phplumb_index::discover::{DiscoverOptions, discover, framework_files};
use phplumb_index::line_index::LineIndex;
use phplumb_index::{Index, Location, member_access_offsets};
use serde::{Deserialize, Serialize};

pub struct BenchArgs {
    pub root: PathBuf,
    pub cases: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    /// A directory relative to the root whose member accesses are measured.
    pub coverage: Option<PathBuf>,
    /// A directory relative to the root whose files are checked for broken
    /// references, whatever their location.
    pub diagnostics: Option<PathBuf>,
    pub exclude: Vec<String>,
}

impl BenchArgs {
    pub fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut root = None;
        let mut cases = None;
        let mut cache = None;
        let mut coverage = None;
        let mut diagnostics = None;
        let mut exclude = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--root" => root = Some(PathBuf::from(value()?)),
                "--cases" => cases = Some(PathBuf::from(value()?)),
                "--cache" => cache = Some(PathBuf::from(value()?)),
                "--coverage" => coverage = Some(PathBuf::from(value()?)),
                "--diagnostics" => diagnostics = Some(PathBuf::from(value()?)),
                "--exclude" => exclude.push(value()?.clone()),
                other => bail!("unknown bench option {other}"),
            }
        }
        Ok(BenchArgs {
            root: root.context("bench needs --root <DIR>")?,
            cases,
            cache,
            coverage,
            diagnostics,
            exclude,
        })
    }
}

/// Workspace symbol queries timed by every run: an exact name, a prefix, camel
/// humps, a name with many members, and one that matches nothing.
const SYMBOL_QUERIES: [&str; 5] = ["node", "EntityStorage", "gEnt", "load", "zzqqxx"];

#[derive(Deserialize)]
struct CaseFile {
    #[serde(default, rename = "case")]
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    kind: String,
    at: String,
    #[serde(default)]
    expect: Vec<String>,
}

#[derive(Serialize)]
pub struct Report {
    files: usize,
    declarations: usize,
    files_with_parse_errors: usize,
    cold_index_ms: u128,
    /// With `--cache`: a build from the cache written after the cold build, then
    /// updates of the first file and of the first 1,000 files.
    #[serde(skip_serializing_if = "Option::is_none")]
    warm_start_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    update_one_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    update_1000_ms: Option<u128>,
    /// The slowest definition case, when the case file has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    definition_ms: Option<u128>,
    /// The slowest implementation case, when the case file has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    implementation_ms: Option<u128>,
    /// The slowest references case, when the case file has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    references_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<Coverage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<Diagnostics>,
    /// The slowest of the workspace symbol queries in `SYMBOL_QUERIES`.
    symbols_ms: u128,
    peak_rss_mb: u64,
    cases_passed: usize,
    cases_failed: usize,
    failures: Vec<Failure>,
}

/// Broken references found under a directory: each as `path:line:column message`,
/// and the slowest file.
#[derive(Serialize)]
struct Diagnostics {
    files: usize,
    count: usize,
    slowest_file_ms: u128,
    entries: Vec<String>,
}

/// How many member accesses under a directory reach at least one definition, and
/// how long asking for all of them took.
#[derive(Serialize)]
struct Coverage {
    member_accesses: usize,
    resolved: usize,
    percent: f64,
    elapsed_ms: u128,
}

#[derive(Serialize)]
struct Failure {
    name: String,
    expected: Vec<String>,
    actual: Vec<String>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.cases_failed == 0
    }
}

pub fn run(args: &BenchArgs) -> anyhow::Result<Report> {
    let root = args.root.canonicalize()?;
    let opts = DiscoverOptions {
        exclude: args.exclude.clone(),
        ..DiscoverOptions::default()
    };
    let (mut index, stats) = Index::build(&root, &opts)?;

    let mut report = Report {
        files: stats.files,
        declarations: stats.declarations,
        files_with_parse_errors: stats.files_with_parse_errors,
        cold_index_ms: stats.elapsed_ms,
        warm_start_ms: None,
        update_one_ms: None,
        update_1000_ms: None,
        definition_ms: None,
        implementation_ms: None,
        references_ms: None,
        coverage: None,
        diagnostics: None,
        symbols_ms: 0,
        peak_rss_mb: 0,
        cases_passed: 0,
        cases_failed: 0,
        failures: Vec::new(),
    };

    if let Some(cache) = &args.cache {
        index.save_cache(cache)?;
        drop(index);
        let (warm, warm_stats) = Index::build_with_cache(&root, &opts, cache)?;
        index = warm;
        report.warm_start_ms = Some(warm_stats.elapsed_ms);
        let paths = discover(&root, &opts)?;
        report.update_one_ms = Some(index.update(&paths[..paths.len().min(1)], &[]).elapsed_ms);
        report.update_1000_ms = Some(
            index
                .update(&paths[..paths.len().min(1000)], &[])
                .elapsed_ms,
        );
    }
    if let Some(dir) = &args.coverage {
        report.coverage = Some(coverage(&mut index, &root, &root.join(dir), &opts)?);
    }
    if let Some(dir) = &args.diagnostics {
        report.diagnostics = Some(diagnostics(&index, &root, &root.join(dir), &opts)?);
    }
    for query in SYMBOL_QUERIES {
        let started = std::time::Instant::now();
        index.workspace_symbols(query, 100);
        report.symbols_ms = report.symbols_ms.max(started.elapsed().as_millis());
    }
    report.peak_rss_mb = peak_rss_bytes() / (1024 * 1024);

    let Some(cases_path) = &args.cases else {
        return Ok(report);
    };
    let text = std::fs::read_to_string(cases_path)
        .with_context(|| format!("reading {}", cases_path.display()))?;
    let file: CaseFile = toml::from_str(&text)?;
    for case in file.cases {
        let expected: BTreeSet<String> = case.expect.iter().cloned().collect();
        let actual = match case.kind.as_str() {
            "definition" => {
                let started = std::time::Instant::now();
                let found = query_case(&mut index, &root, &case.at, Index::definition);
                let elapsed = started.elapsed().as_millis();
                report.definition_ms = report.definition_ms.max(Some(elapsed));
                found
            }
            "implementation" => {
                let started = std::time::Instant::now();
                let found = query_case(&mut index, &root, &case.at, Index::implementations);
                let elapsed = started.elapsed().as_millis();
                report.implementation_ms = report.implementation_ms.max(Some(elapsed));
                found
            }
            "references" => {
                let started = std::time::Instant::now();
                let found = query_case(&mut index, &root, &case.at, |index, path, source, at| {
                    index.references(path, source, at, false)
                });
                let elapsed = started.elapsed().as_millis();
                report.references_ms = report.references_ms.max(Some(elapsed));
                found
            }
            other => Err(anyhow::anyhow!("unsupported case kind {other}")),
        };
        match actual {
            Ok(actual) if actual == expected => report.cases_passed += 1,
            Ok(actual) => {
                report.cases_failed += 1;
                report.failures.push(Failure {
                    name: case.name,
                    expected: expected.into_iter().collect(),
                    actual: actual.into_iter().collect(),
                });
            }
            Err(err) => {
                report.cases_failed += 1;
                report.failures.push(Failure {
                    name: case.name,
                    expected: expected.into_iter().collect(),
                    actual: vec![format!("error: {err:#}")],
                });
            }
        }
    }
    Ok(report)
}

fn diagnostics(
    index: &Index,
    root: &Path,
    dir: &Path,
    opts: &DiscoverOptions,
) -> anyhow::Result<Diagnostics> {
    let dir = dir.canonicalize()?;
    let mut report = Diagnostics {
        files: 0,
        count: 0,
        slowest_file_ms: 0,
        entries: Vec::new(),
    };
    let mut files = discover(root, opts)?;
    files.extend(framework_files(root, opts)?);
    for path in files.into_iter().filter(|p| p.starts_with(&dir)) {
        let source = std::fs::read(&path)?;
        let started = std::time::Instant::now();
        let found = index.diagnostics(&path, &source);
        report.slowest_file_ms = report.slowest_file_ms.max(started.elapsed().as_millis());
        report.files += 1;
        report.count += found.len();
        let lines = LineIndex::new(&source);
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        for diagnostic in found {
            let (line, col) = lines.position(&source, diagnostic.start);
            report.entries.push(format!(
                "{relative}:{}:{} {}",
                line + 1,
                col + 1,
                diagnostic.message
            ));
        }
    }
    Ok(report)
}

fn coverage(
    index: &mut Index,
    root: &Path,
    dir: &Path,
    opts: &DiscoverOptions,
) -> anyhow::Result<Coverage> {
    let started = std::time::Instant::now();
    let dir = dir.canonicalize()?;
    let mut member_accesses = 0;
    let mut resolved = 0;
    for path in discover(root, opts)?
        .into_iter()
        .filter(|p| p.starts_with(&dir))
    {
        let source = std::fs::read(&path)?;
        for offset in member_access_offsets(&source) {
            member_accesses += 1;
            if !index.definition(&path, &source, offset).is_empty() {
                resolved += 1;
            }
        }
    }
    let percent = if member_accesses == 0 {
        0.0
    } else {
        (resolved as f64 * 1000.0 / member_accesses as f64).round() / 10.0
    };
    Ok(Coverage {
        member_accesses,
        resolved,
        percent,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// Runs `query` at the case position and renders its targets as case positions:
/// relative to the root, or under `stubs/` for a stub file.
fn query_case(
    index: &mut Index,
    root: &Path,
    at: &str,
    query: fn(&mut Index, &Path, &[u8], u32) -> Vec<Location>,
) -> anyhow::Result<BTreeSet<String>> {
    let (path, line, col) = parse_position(root, at)?;
    let source = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let offset = LineIndex::new(&source)
        .offset(&source, line - 1, col - 1)
        .with_context(|| format!("{at} is outside the file"))?;
    let found = query(index, &path, &source, offset);
    let stubs = index.stubs_dir();
    found
        .into_iter()
        .map(|location| {
            let text = std::fs::read(&location.path)?;
            let (line, col) = LineIndex::new(&text).position(&text, location.start);
            let relative = match stubs.and_then(|dir| location.path.strip_prefix(dir).ok()) {
                Some(stub) => Path::new("stubs").join(stub),
                None => location
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&location.path)
                    .to_path_buf(),
            };
            Ok(format!("{}:{}:{}", relative.display(), line + 1, col + 1))
        })
        .collect()
}

fn parse_position(root: &Path, at: &str) -> anyhow::Result<(PathBuf, u32, u32)> {
    let mut parts = at.rsplitn(3, ':');
    let col = parts.next().context("missing column")?.parse::<u32>()?;
    let line = parts.next().context("missing line")?.parse::<u32>()?;
    let path = parts.next().context("missing path")?;
    if line == 0 || col == 0 {
        bail!("{at}: lines and columns are 1-based");
    }
    Ok((root.join(path), line, col))
}

/// Peak resident memory of this process.
fn peak_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct it is given and only reads RUSAGE_SELF.
    let usage = unsafe {
        if libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) != 0 {
            return 0;
        }
        usage.assume_init()
    };
    let max_rss = usage.ru_maxrss as u64;
    // ru_maxrss is in bytes on macOS and in KiB on Linux.
    if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss * 1024
    }
}
