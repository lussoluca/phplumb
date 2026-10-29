//! The LSP server: initialization, indexing, file watching and request handling.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeWatchedFiles, LogMessage, Notification as _, Progress, ShowMessage,
};
use lsp_types::request::{
    CallHierarchyIncomingCalls, CallHierarchyOutgoingCalls, CallHierarchyPrepare,
    DocumentSymbolRequest, GotoDefinition, GotoImplementation, GotoImplementationParams,
    HoverRequest, References, RegisterCapability, Request as _, Shutdown, TypeHierarchyPrepare,
    TypeHierarchySubtypes, TypeHierarchySupertypes, WorkDoneProgressCreate, WorkspaceSymbolRequest,
};
use lsp_types::request::{DocumentDiagnosticRequest, WorkspaceDiagnosticRefresh};
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CallHierarchyServerCapability, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, FileChangeType, FileEvent, FileSystemWatcher, GlobPattern,
    GotoDefinitionParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
    ImplementationProviderCapability, InitializeParams, Location, LogMessageParams, MarkupContent,
    MarkupKind, MessageType, NumberOrString, OneOf, Position, ProgressParams, ProgressParamsValue,
    Range, ReferenceParams, Registration, RegistrationParams, ServerCapabilities,
    ShowMessageParams, SymbolKind, TypeHierarchyItem, TypeHierarchyPrepareParams,
    TypeHierarchySubtypesParams, TypeHierarchySupertypesParams, WatchKind, WorkDoneProgress,
    WorkDoneProgressBegin, WorkDoneProgressCreateParams, WorkDoneProgressEnd, WorkspaceSymbol,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use lsp_types::{
    DiagnosticOptions, DiagnosticServerCapabilities, DiagnosticSeverity, DocumentDiagnosticParams,
    DocumentDiagnosticReport, DocumentDiagnosticReportResult, FullDocumentDiagnosticReport,
    RelatedFullDocumentDiagnosticReport,
};
use phplumb_index::diagnostics::Severity;
use phplumb_index::discover::{DiscoverOptions, PHP_EXTENSION};
use phplumb_index::frameworks::{self, Detection, FrameworkSettings};
use phplumb_index::line_index::LineIndex;
use phplumb_index::summary::{MemberKind, SymbolKind as IndexSymbolKind};
use phplumb_index::symbols::{self, Kind, OutlineSymbol};
use phplumb_index::{CallItem, CallKind, ClassRef, Index, IndexStats};
use serde::Deserialize;
use serde_json::json;

use crate::convert::{path_to_uri, uri_to_path};

const PROGRESS_TOKEN: &str = "phplumb/indexing";
const WATCH_REGISTRATION: &str = "phplumb/watch";
/// Sent once at startup: `{ "extensions": ["php", "module"] }`.
const PHP_EXTENSIONS_NOTIFICATION: &str = "phplumb/phpExtensions";
const CACHE_FILE: &str = "index-v1.bin";
/// How long to wait for more file events before applying a batch. Branch switches
/// and agents report changes in many small notifications.
const DEBOUNCE: Duration = Duration::from_millis(50);

/// Parsed from `initializationOptions`:
/// `{ "root": "src/drupal", "exclude": ["vendor/rector/rector/**"],
/// "cacheDir": "/path/to/storage", "diagnostics": true, "frameworks": {"drupal": "auto", "symfony": "auto"} }`.
#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerSettings {
    /// The project folder, relative to the workspace folder; empty for the workspace
    /// folder itself.
    pub root: PathBuf,
    /// Globs relative to the project folder.
    pub exclude: Vec<String>,
    pub cache_dir: Option<PathBuf>,
    /// Whether broken references are reported.
    pub diagnostics: bool,
    /// Which framework modules run.
    pub frameworks: FrameworkSettings,
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings {
            root: PathBuf::new(),
            exclude: Vec::new(),
            cache_dir: None,
            diagnostics: true,
            frameworks: FrameworkSettings::default(),
        }
    }
}

/// Serves LSP over stdin and stdout until the client sends `exit`.
pub fn serve() -> anyhow::Result<()> {
    let (connection, io_threads) = Connection::stdio();
    run(connection)?;
    io_threads.join()?;
    Ok(())
}

pub fn run(connection: Connection) -> anyhow::Result<()> {
    let (id, raw_params) = connection.initialize_start()?;
    // lsp-types 0.97 reads `workspace.diagnostic`; the protocol names it `diagnostics`.
    let refreshes_diagnostics = ["diagnostics", "diagnostic"].iter().any(|key| {
        raw_params["capabilities"]["workspace"][key]["refreshSupport"].as_bool() == Some(true)
    });
    let params: InitializeParams = serde_json::from_value(raw_params)?;
    let capabilities = ServerCapabilities {
        definition_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
        references_provider: Some(OneOf::Left(true)),
        call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(DiagnosticOptions {
            identifier: Some("phplumb".to_string()),
            inter_file_dependencies: true,
            workspace_diagnostics: false,
            work_done_progress_options: Default::default(),
        })),
        document_symbol_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    // lsp-types 0.97 has no field for the type hierarchy capability.
    let mut capabilities = serde_json::to_value(capabilities)?;
    capabilities["typeHierarchyProvider"] = json!(true);
    connection.initialize_finish(
        id,
        json!({
            "capabilities": capabilities,
            "serverInfo": {"name": "phplumb", "version": env!("CARGO_PKG_VERSION")},
        }),
    )?;

    let settings: ServerSettings = params
        .initialization_options
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let reports_progress = params
        .capabilities
        .window
        .as_ref()
        .and_then(|window| window.work_done_progress)
        .unwrap_or(false);
    let watches_files = params
        .capabilities
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.did_change_watched_files)
        .and_then(|watched| watched.dynamic_registration)
        .unwrap_or(false);

    let (root, root_warning) = project_root(workspace_root(&params), &settings.root);
    let mut server = Server {
        connection: &connection,
        root,
        opts: DiscoverOptions {
            exclude: settings.exclude,
            frameworks: settings.frameworks,
            ..DiscoverOptions::default()
        },
        cache: settings.cache_dir.map(|dir| dir.join(CACHE_FILE)),
        index: None,
        detection: None,
        pending: VecDeque::new(),
        diagnostics: settings.diagnostics,
        refreshes_diagnostics,
        refreshes_sent: 0,
    };
    if let Some(warning) = root_warning {
        server.show(MessageType::WARNING, warning)?;
    }
    server.detection = server
        .root
        .as_deref()
        .map(|root| frameworks::detect(root, &server.opts.frameworks));
    let php_extensions = server
        .detection
        .as_ref()
        .map_or_else(|| vec![PHP_EXTENSION], Detection::php_extensions);
    if watches_files {
        server.register_watchers(&php_extensions)?;
    }
    server.announce_php_extensions(&php_extensions)?;
    // Requests that arrive while indexing wait in the connection's queue.
    server.rebuild(reports_progress)?;
    server.save_cache()?;
    server.main_loop()
}

/// The first workspace folder, or the deprecated `rootUri` when there are none.
fn workspace_root(params: &InitializeParams) -> Option<PathBuf> {
    if let Some(folder) = params.workspace_folders.as_ref().and_then(|f| f.first()) {
        return uri_to_path(&folder.uri);
    }
    #[allow(deprecated)]
    params.root_uri.as_ref().and_then(uri_to_path)
}

/// The folder to index: `setting` below `workspace`, or `workspace` itself when
/// `setting` is empty. A setting that names no folder falls back to `workspace`, with
/// a warning for the user.
fn project_root(workspace: Option<PathBuf>, setting: &Path) -> (Option<PathBuf>, Option<String>) {
    let Some(workspace) = workspace else {
        return (None, None);
    };
    if setting.as_os_str().is_empty() {
        return (Some(workspace), None);
    }
    let root = workspace.join(setting);
    if root.is_dir() {
        return (Some(root), None);
    }
    let warning = format!(
        "phplumb.root \"{}\" is not a folder of the workspace; indexing the whole workspace.",
        setting.display()
    );
    (Some(workspace), Some(warning))
}

struct Server<'a> {
    connection: &'a Connection,
    root: Option<PathBuf>,
    opts: DiscoverOptions,
    cache: Option<PathBuf>,
    index: Option<Index>,
    /// The framework detection made at start, used by the first build.
    detection: Option<Detection>,
    /// Messages read while collecting file events, handled before the queue.
    pending: VecDeque<Message>,
    /// Whether broken references are reported.
    diagnostics: bool,
    /// Whether the client re-pulls diagnostics when asked.
    refreshes_diagnostics: bool,
    refreshes_sent: u64,
}

impl Server<'_> {
    fn main_loop(&mut self) -> anyhow::Result<()> {
        loop {
            let message = match self.pending.pop_front() {
                Some(message) => message,
                None => match self.connection.receiver.recv() {
                    Ok(message) => message,
                    Err(_) => return Ok(()),
                },
            };
            match message {
                Message::Request(request) => {
                    if request.method == Shutdown::METHOD {
                        self.save_cache()?;
                    }
                    if self.connection.handle_shutdown(&request)? {
                        return Ok(());
                    }
                    self.handle_request(request)?;
                }
                Message::Notification(notification)
                    if notification.method == DidChangeWatchedFiles::METHOD =>
                {
                    let mut events = file_events(notification);
                    self.collect_more_events(&mut events);
                    self.apply_events(events)?;
                }
                _ => {}
            }
        }
    }

    fn handle_request(&mut self, request: Request) -> anyhow::Result<()> {
        let response = match request.method.as_str() {
            GotoDefinition::METHOD => {
                match serde_json::from_value::<GotoDefinitionParams>(request.params) {
                    Ok(params) => {
                        Response::new_ok(request.id, definition(self.index.as_mut(), params))
                    }
                    Err(err) => Response::new_err(
                        request.id,
                        ErrorCode::InvalidParams as i32,
                        err.to_string(),
                    ),
                }
            }
            GotoImplementation::METHOD => {
                match serde_json::from_value::<GotoImplementationParams>(request.params) {
                    Ok(params) => {
                        Response::new_ok(request.id, implementation(self.index.as_mut(), params))
                    }
                    Err(err) => invalid(request.id, err),
                }
            }
            TypeHierarchyPrepare::METHOD => {
                match serde_json::from_value::<TypeHierarchyPrepareParams>(request.params) {
                    Ok(params) => Response::new_ok(
                        request.id,
                        prepare_type_hierarchy(self.index.as_mut(), params),
                    ),
                    Err(err) => invalid(request.id, err),
                }
            }
            TypeHierarchySupertypes::METHOD => {
                match serde_json::from_value::<TypeHierarchySupertypesParams>(request.params) {
                    Ok(params) => Response::new_ok(
                        request.id,
                        hierarchy_items(self.index.as_ref(), &params.item, Direction::Up),
                    ),
                    Err(err) => invalid(request.id, err),
                }
            }
            TypeHierarchySubtypes::METHOD => {
                match serde_json::from_value::<TypeHierarchySubtypesParams>(request.params) {
                    Ok(params) => Response::new_ok(
                        request.id,
                        hierarchy_items(self.index.as_ref(), &params.item, Direction::Down),
                    ),
                    Err(err) => invalid(request.id, err),
                }
            }
            References::METHOD => match serde_json::from_value::<ReferenceParams>(request.params) {
                Ok(params) => Response::new_ok(request.id, references(self.index.as_mut(), params)),
                Err(err) => invalid(request.id, err),
            },
            CallHierarchyPrepare::METHOD => {
                match serde_json::from_value::<CallHierarchyPrepareParams>(request.params) {
                    Ok(params) => Response::new_ok(
                        request.id,
                        prepare_call_hierarchy(self.index.as_mut(), params),
                    ),
                    Err(err) => invalid(request.id, err),
                }
            }
            CallHierarchyIncomingCalls::METHOD => {
                match serde_json::from_value::<CallHierarchyIncomingCallsParams>(request.params) {
                    Ok(params) => {
                        Response::new_ok(request.id, incoming_calls(self.index.as_mut(), params))
                    }
                    Err(err) => invalid(request.id, err),
                }
            }
            CallHierarchyOutgoingCalls::METHOD => {
                match serde_json::from_value::<CallHierarchyOutgoingCallsParams>(request.params) {
                    Ok(params) => {
                        Response::new_ok(request.id, outgoing_calls(self.index.as_mut(), params))
                    }
                    Err(err) => invalid(request.id, err),
                }
            }
            DocumentSymbolRequest::METHOD => {
                match serde_json::from_value::<DocumentSymbolParams>(request.params) {
                    Ok(params) => Response::new_ok(request.id, document_symbols(params)),
                    Err(err) => invalid(request.id, err),
                }
            }
            WorkspaceSymbolRequest::METHOD => {
                match serde_json::from_value::<WorkspaceSymbolParams>(request.params) {
                    Ok(params) => {
                        Response::new_ok(request.id, workspace_symbols(self.index.as_ref(), params))
                    }
                    Err(err) => invalid(request.id, err),
                }
            }
            DocumentDiagnosticRequest::METHOD => {
                match serde_json::from_value::<DocumentDiagnosticParams>(request.params) {
                    Ok(params) => {
                        let items = if self.diagnostics {
                            document_diagnostics(self.index.as_ref(), params)
                        } else {
                            Vec::new()
                        };
                        Response::new_ok(
                            request.id,
                            DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(
                                RelatedFullDocumentDiagnosticReport {
                                    related_documents: None,
                                    full_document_diagnostic_report: FullDocumentDiagnosticReport {
                                        result_id: None,
                                        items,
                                    },
                                },
                            )),
                        )
                    }
                    Err(err) => invalid(request.id, err),
                }
            }
            HoverRequest::METHOD => match serde_json::from_value::<HoverParams>(request.params) {
                Ok(params) => Response::new_ok(request.id, hover(self.index.as_mut(), params)),
                Err(err) => {
                    Response::new_err(request.id, ErrorCode::InvalidParams as i32, err.to_string())
                }
            },
            _ => Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("unhandled method {}", request.method),
            ),
        };
        self.connection.sender.send(Message::Response(response))?;
        Ok(())
    }

    /// Adds every file event that arrives within the debounce window. The first other
    /// message ends the window and is handled after the batch is applied.
    fn collect_more_events(&mut self, events: &mut Vec<FileEvent>) {
        while let Ok(message) = self.connection.receiver.recv_timeout(DEBOUNCE) {
            match message {
                Message::Notification(notification)
                    if notification.method == DidChangeWatchedFiles::METHOD =>
                {
                    events.extend(file_events(notification));
                }
                other => {
                    self.pending.push_back(other);
                    return;
                }
            }
        }
    }

    /// Applies one batch of file events. A change to Composer's `installed.json`
    /// changes which vendor files belong to the index, so it rebuilds everything.
    fn apply_events(&mut self, events: Vec<FileEvent>) -> anyhow::Result<()> {
        let mut changed = BTreeSet::new();
        let mut removed = BTreeSet::new();
        let mut rebuild = false;
        for event in events {
            let Some(path) = uri_to_path(&event.uri) else {
                continue;
            };
            if path.ends_with("vendor/composer/installed.json") {
                rebuild = true;
            } else if event.typ == FileChangeType::DELETED {
                removed.insert(path);
            } else {
                changed.insert(path);
            }
        }
        if rebuild {
            return self.rebuild(false);
        }
        let Some(index) = self.index.as_mut() else {
            return Ok(());
        };
        let changed: Vec<PathBuf> = changed.into_iter().collect();
        let removed: Vec<PathBuf> = removed.into_iter().collect();
        let stats = index.update(&changed, &removed);
        // YAML, Twig and XML files define what the frameworks check without being
        // indexed themselves.
        let framework_files = changed
            .iter()
            .chain(&removed)
            .any(|path| phplumb_index::filter::is_framework_file(path));
        if stats.reparsed == 0 && stats.removed == 0 {
            return if framework_files {
                self.refresh_diagnostics()
            } else {
                Ok(())
            };
        }
        self.log_info(format!(
            "Reindexed {} files, removed {} in {} ms",
            stats.reparsed, stats.removed, stats.elapsed_ms
        ))?;
        self.refresh_diagnostics()
    }

    /// Asks the client to pull diagnostics again, after the index changed.
    fn refresh_diagnostics(&mut self) -> anyhow::Result<()> {
        if !(self.diagnostics && self.refreshes_diagnostics) {
            return Ok(());
        }
        self.refreshes_sent += 1;
        self.connection.sender.send(Message::Request(Request::new(
            RequestId::from(format!(
                "phplumb/diagnostics-refresh-{}",
                self.refreshes_sent
            )),
            WorkspaceDiagnosticRefresh::METHOD.to_string(),
            (),
        )))?;
        Ok(())
    }

    /// Tells the client which extensions PHP files have, so it can open them as PHP.
    fn announce_php_extensions(&self, extensions: &[&str]) -> anyhow::Result<()> {
        self.connection
            .sender
            .send(Message::Notification(Notification::new(
                PHP_EXTENSIONS_NOTIFICATION.to_string(),
                json!({"extensions": extensions}),
            )))?;
        Ok(())
    }

    /// Watches files with `php_extensions`, Composer's installed packages, YAML files,
    /// Doctrine XML mappings and directories.
    fn register_watchers(&self, php_extensions: &[&str]) -> anyhow::Result<()> {
        let watcher = |pattern: String, kind: Option<WatchKind>| FileSystemWatcher {
            glob_pattern: GlobPattern::String(pattern),
            kind,
        };
        let php = match php_extensions {
            [extension] => format!("**/*.{extension}"),
            extensions => format!("**/*.{{{}}}", extensions.join(",")),
        };
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![
                watcher(php, None),
                watcher("**/vendor/composer/installed.json".to_string(), None),
                // Framework modules read YAML files, such as Drupal's services and
                // Symfony's routes.
                watcher("**/*.{yml,yaml}".to_string(), None),
                // Doctrine mappings, which the Symfony module reads.
                watcher("**/*.orm.xml".to_string(), None),
                // Creating, moving or deleting a directory reports only the directory.
                watcher(
                    "**/*".to_string(),
                    Some(WatchKind::Create | WatchKind::Delete),
                ),
            ],
        };
        let registration = Registration {
            id: WATCH_REGISTRATION.to_string(),
            method: DidChangeWatchedFiles::METHOD.to_string(),
            register_options: Some(serde_json::to_value(options)?),
        };
        self.connection.sender.send(Message::Request(Request::new(
            RequestId::from(WATCH_REGISTRATION.to_string()),
            RegisterCapability::METHOD.to_string(),
            RegistrationParams {
                registrations: vec![registration],
            },
        )))?;
        Ok(())
    }

    /// Builds the index from scratch, from the cache when there is one.
    fn rebuild(&mut self, reports_progress: bool) -> anyhow::Result<()> {
        let Some(root) = self.root.clone() else {
            return Ok(());
        };
        let token = NumberOrString::String(PROGRESS_TOKEN.to_string());
        if reports_progress {
            self.connection.sender.send(Message::Request(Request::new(
                RequestId::from(PROGRESS_TOKEN.to_string()),
                WorkDoneProgressCreate::METHOD.to_string(),
                WorkDoneProgressCreateParams {
                    token: token.clone(),
                },
            )))?;
            self.send_progress(
                &token,
                WorkDoneProgress::Begin(WorkDoneProgressBegin {
                    title: "PHPlumb".to_string(),
                    message: Some("Indexing".to_string()),
                    ..WorkDoneProgressBegin::default()
                }),
            )?;
        }

        let detection = self
            .detection
            .take()
            .unwrap_or_else(|| frameworks::detect(&root, &self.opts.frameworks));
        let (index, stats) =
            Index::build_with(&root, &self.opts, self.cache.as_deref(), &detection)?;
        let frameworks = index.active_frameworks();
        self.index = Some(index);
        let summary = summary_message(&stats);
        let frameworks = match frameworks.is_empty() {
            true => String::new(),
            false => format!("; frameworks: {}", frameworks.join(", ")),
        };
        self.log_info(format!(
            "{summary} ({} declarations, {} files from cache{frameworks})",
            stats.declarations, stats.reused
        ))?;

        if reports_progress {
            self.send_progress(
                &token,
                WorkDoneProgress::End(WorkDoneProgressEnd {
                    message: Some(summary),
                }),
            )?;
        }
        self.refresh_diagnostics()
    }

    /// Writes the cache when there is one. A failure is logged, not fatal: the next
    /// start parses the changed files again.
    fn save_cache(&self) -> anyhow::Result<()> {
        let (Some(cache), Some(index)) = (&self.cache, &self.index) else {
            return Ok(());
        };
        match index.save_cache(cache) {
            Ok(()) => Ok(()),
            Err(err) => self.log(
                MessageType::WARNING,
                format!("Could not write the index cache: {err:#}"),
            ),
        }
    }

    fn log_info(&self, message: String) -> anyhow::Result<()> {
        self.log(MessageType::INFO, message)
    }

    /// Shows a message to the user.
    fn show(&self, typ: MessageType, message: String) -> anyhow::Result<()> {
        self.connection
            .sender
            .send(Message::Notification(Notification::new(
                ShowMessage::METHOD.to_string(),
                ShowMessageParams { typ, message },
            )))?;
        Ok(())
    }

    /// Sends a message to the client's log.
    fn log(&self, typ: MessageType, message: String) -> anyhow::Result<()> {
        self.connection
            .sender
            .send(Message::Notification(Notification::new(
                LogMessage::METHOD.to_string(),
                LogMessageParams { typ, message },
            )))?;
        Ok(())
    }

    fn send_progress(
        &self,
        token: &NumberOrString,
        progress: WorkDoneProgress,
    ) -> anyhow::Result<()> {
        self.connection
            .sender
            .send(Message::Notification(Notification::new(
                Progress::METHOD.to_string(),
                ProgressParams {
                    token: token.clone(),
                    value: ProgressParamsValue::WorkDone(progress),
                },
            )))?;
        Ok(())
    }
}

fn file_events(notification: Notification) -> Vec<FileEvent> {
    serde_json::from_value::<DidChangeWatchedFilesParams>(notification.params)
        .map(|params| params.changes)
        .unwrap_or_default()
}

fn summary_message(stats: &IndexStats) -> String {
    format!("Indexed {} files in {} ms", stats.files, stats.elapsed_ms)
}

/// The file and byte offset a text document position names, read from disk so the
/// query sees what the agent last wrote.
fn cursor(position: &lsp_types::TextDocumentPositionParams) -> Option<(PathBuf, Vec<u8>, u32)> {
    let path = uri_to_path(&position.text_document.uri)?;
    let source = std::fs::read(&path).ok()?;
    let offset = LineIndex::new(&source).offset(
        &source,
        position.position.line,
        position.position.character,
    )?;
    Some((path, source, offset))
}

fn definition(index: Option<&mut Index>, params: GotoDefinitionParams) -> Vec<Location> {
    let (Some(index), Some((path, source, offset))) =
        (index, cursor(&params.text_document_position_params))
    else {
        return Vec::new();
    };
    to_locations(&index.definition(&path, &source, offset))
}

fn implementation(index: Option<&mut Index>, params: GotoImplementationParams) -> Vec<Location> {
    let (Some(index), Some((path, source, offset))) =
        (index, cursor(&params.text_document_position_params))
    else {
        return Vec::new();
    };
    to_locations(&index.implementations(&path, &source, offset))
}

fn references(index: Option<&mut Index>, params: ReferenceParams) -> Vec<Location> {
    let (Some(index), Some((path, source, offset))) =
        (index, cursor(&params.text_document_position))
    else {
        return Vec::new();
    };
    let include = params.context.include_declaration;
    to_locations(&index.references(&path, &source, offset, include))
}

fn prepare_call_hierarchy(
    index: Option<&mut Index>,
    params: CallHierarchyPrepareParams,
) -> Option<Vec<CallHierarchyItem>> {
    let index = index?;
    let (path, source, offset) = cursor(&params.text_document_position_params)?;
    let items: Vec<CallHierarchyItem> = index
        .call_items(&path, &source, offset)
        .iter()
        .filter_map(call_item)
        .collect();
    (!items.is_empty()).then_some(items)
}

fn incoming_calls(
    index: Option<&mut Index>,
    params: CallHierarchyIncomingCallsParams,
) -> Option<Vec<CallHierarchyIncomingCall>> {
    let index = index?;
    let item = from_call_item(&params.item)?;
    Some(
        index
            .incoming_calls(&item)
            .iter()
            .filter_map(|(caller, ranges)| {
                Some(CallHierarchyIncomingCall {
                    from: call_item(caller)?,
                    from_ranges: ranges_in(&caller.path, ranges)?,
                })
            })
            .collect(),
    )
}

fn outgoing_calls(
    index: Option<&mut Index>,
    params: CallHierarchyOutgoingCallsParams,
) -> Option<Vec<CallHierarchyOutgoingCall>> {
    let index = index?;
    let item = from_call_item(&params.item)?;
    Some(
        index
            .outgoing_calls(&item)
            .iter()
            .filter_map(|(callee, ranges)| {
                Some(CallHierarchyOutgoingCall {
                    to: call_item(callee)?,
                    from_ranges: ranges_in(&item.path, ranges)?,
                })
            })
            .collect(),
    )
}

/// An index call item as an LSP item; `data` carries what rebuilding it needs.
fn call_item(item: &CallItem) -> Option<CallHierarchyItem> {
    let ranges = ranges_in(&item.path, &[item.range, item.selection])?;
    let kind = match item.kind {
        CallKind::Function => SymbolKind::FUNCTION,
        CallKind::Method => SymbolKind::METHOD,
        CallKind::File => SymbolKind::FILE,
    };
    let kind_name = match item.kind {
        CallKind::Function => "function",
        CallKind::Method => "method",
        CallKind::File => "file",
    };
    Some(CallHierarchyItem {
        name: item.name.clone(),
        kind,
        tags: None,
        detail: Some(item.detail.clone()),
        uri: path_to_uri(&item.path),
        range: ranges[0],
        selection_range: ranges[1],
        data: Some(json!({
            "path": item.path,
            "range": [item.range.0, item.range.1],
            "selection": [item.selection.0, item.selection.1],
            "kind": kind_name,
            "name": item.name,
            "detail": item.detail,
        })),
    })
}

fn from_call_item(item: &CallHierarchyItem) -> Option<CallItem> {
    let data = item.data.as_ref()?;
    let pair = |key: &str| -> Option<(u32, u32)> {
        let values = data.get(key)?.as_array()?;
        Some((
            values.first()?.as_u64()? as u32,
            values.get(1)?.as_u64()? as u32,
        ))
    };
    let kind = match data.get("kind")?.as_str()? {
        "function" => CallKind::Function,
        "method" => CallKind::Method,
        _ => CallKind::File,
    };
    Some(CallItem {
        name: data.get("name")?.as_str()?.to_string(),
        kind,
        detail: data.get("detail")?.as_str()?.to_string(),
        path: PathBuf::from(data.get("path")?.as_str()?),
        range: pair("range")?,
        selection: pair("selection")?,
    })
}

/// Byte ranges of the file at `path` as LSP ranges.
fn ranges_in(path: &Path, ranges: &[(u32, u32)]) -> Option<Vec<Range>> {
    let text = std::fs::read(path).ok()?;
    let lines = LineIndex::new(&text);
    Some(
        ranges
            .iter()
            .map(|(start, end)| {
                let (start_line, start_col) = lines.position(&text, *start);
                let (end_line, end_col) = lines.position(&text, *end);
                Range::new(
                    Position::new(start_line, start_col),
                    Position::new(end_line, end_col),
                )
            })
            .collect(),
    )
}

/// Broken references in the file, when it is project code.
fn document_diagnostics(
    index: Option<&Index>,
    params: DocumentDiagnosticParams,
) -> Vec<lsp_types::Diagnostic> {
    let (Some(index), Some(path)) = (index, uri_to_path(&params.text_document.uri)) else {
        return Vec::new();
    };
    if !index.is_project_code(&path) {
        return Vec::new();
    }
    let Ok(source) = std::fs::read(&path) else {
        return Vec::new();
    };
    let lines = LineIndex::new(&source);
    index
        .diagnostics(&path, &source)
        .into_iter()
        .map(|found| {
            let (start_line, start_col) = lines.position(&source, found.start);
            let (end_line, end_col) = lines.position(&source, found.end);
            lsp_types::Diagnostic {
                range: Range::new(
                    Position::new(start_line, start_col),
                    Position::new(end_line, end_col),
                ),
                severity: Some(match found.severity {
                    Severity::Error => DiagnosticSeverity::ERROR,
                    Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                source: Some("phplumb".to_string()),
                message: found.message,
                ..lsp_types::Diagnostic::default()
            }
        })
        .collect()
}

/// How many workspace symbols a query returns.
const WORKSPACE_SYMBOL_LIMIT: usize = 100;

fn document_symbols(params: DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
    let path = uri_to_path(&params.text_document.uri)?;
    if phplumb_index::filter::is_framework_file(&path) {
        return None;
    }
    let source = std::fs::read(&path).ok()?;
    let lines = LineIndex::new(&source);
    let range = |(start, end): (u32, u32)| {
        let (start_line, start_col) = lines.position(&source, start);
        let (end_line, end_col) = lines.position(&source, end);
        Range::new(
            Position::new(start_line, start_col),
            Position::new(end_line, end_col),
        )
    };
    fn convert(symbol: &OutlineSymbol, range: &dyn Fn((u32, u32)) -> Range) -> DocumentSymbol {
        #[allow(deprecated)]
        DocumentSymbol {
            name: symbol.name.clone(),
            detail: None,
            kind: symbol_kind(symbol.kind, &symbol.name),
            tags: None,
            deprecated: None,
            range: range(symbol.range),
            selection_range: range(symbol.selection),
            children: Some(
                symbol
                    .children
                    .iter()
                    .map(|child| convert(child, range))
                    .collect(),
            ),
        }
    }
    Some(DocumentSymbolResponse::Nested(
        symbols::outline(&source)
            .iter()
            .map(|symbol| convert(symbol, &range))
            .collect(),
    ))
}

fn workspace_symbols(
    index: Option<&Index>,
    params: WorkspaceSymbolParams,
) -> WorkspaceSymbolResponse {
    let Some(index) = index else {
        return WorkspaceSymbolResponse::Nested(Vec::new());
    };
    let mut files: HashMap<PathBuf, Option<(Vec<u8>, LineIndex)>> = HashMap::new();
    let found = index
        .workspace_symbols(&params.query, WORKSPACE_SYMBOL_LIMIT)
        .into_iter()
        .filter_map(|found| {
            let file = files
                .entry(found.location.path.clone())
                .or_insert_with(|| {
                    let text = std::fs::read(&found.location.path).ok()?;
                    let lines = LineIndex::new(&text);
                    Some((text, lines))
                })
                .as_ref()?;
            let (start_line, start_col) = file.1.position(&file.0, found.location.start);
            let (end_line, end_col) = file.1.position(&file.0, found.location.end);
            Some(WorkspaceSymbol {
                kind: symbol_kind(found.kind, &found.name),
                name: found.name,
                tags: None,
                container_name: found.container,
                location: OneOf::Left(Location::new(
                    path_to_uri(&found.location.path),
                    Range::new(
                        Position::new(start_line, start_col),
                        Position::new(end_line, end_col),
                    ),
                )),
                data: None,
            })
        })
        .collect();
    WorkspaceSymbolResponse::Nested(found)
}

fn symbol_kind(kind: Kind, name: &str) -> SymbolKind {
    match kind {
        Kind::Declaration(IndexSymbolKind::Class) => SymbolKind::CLASS,
        Kind::Declaration(IndexSymbolKind::Interface) => SymbolKind::INTERFACE,
        Kind::Declaration(IndexSymbolKind::Trait) => SymbolKind::STRUCT,
        Kind::Declaration(IndexSymbolKind::Enum) => SymbolKind::ENUM,
        Kind::Declaration(IndexSymbolKind::Function) => SymbolKind::FUNCTION,
        Kind::Declaration(IndexSymbolKind::Constant) => SymbolKind::CONSTANT,
        Kind::Member(MemberKind::Method) if name.eq_ignore_ascii_case("__construct") => {
            SymbolKind::CONSTRUCTOR
        }
        Kind::Member(MemberKind::Method) => SymbolKind::METHOD,
        Kind::Member(MemberKind::Property) => SymbolKind::PROPERTY,
        Kind::Member(MemberKind::Constant) => SymbolKind::CONSTANT,
        Kind::Member(MemberKind::EnumCase) => SymbolKind::ENUM_MEMBER,
    }
}

/// One type hierarchy item per class-like the cursor resolves to.
fn prepare_type_hierarchy(
    index: Option<&mut Index>,
    params: TypeHierarchyPrepareParams,
) -> Option<Vec<TypeHierarchyItem>> {
    let index = index?;
    let (path, source, offset) = cursor(&params.text_document_position_params)?;
    let targets = index.targets(&path, &source, offset);
    let items: Vec<TypeHierarchyItem> = targets
        .iter()
        .filter_map(|target| index.class_declared_at(&target.path, target.start))
        .filter_map(|class| hierarchy_item(&class))
        .collect();
    (!items.is_empty()).then_some(items)
}

enum Direction {
    Up,
    Down,
}

fn hierarchy_items(
    index: Option<&Index>,
    item: &TypeHierarchyItem,
    direction: Direction,
) -> Option<Vec<TypeHierarchyItem>> {
    let index = index?;
    let fqn = item.data.as_ref()?.get("fqn")?.as_str()?;
    let classes = match direction {
        Direction::Up => index.supertypes(fqn),
        Direction::Down => index.subtypes(fqn),
    };
    Some(
        classes
            .iter()
            .filter(|class| !index.is_stub(class.path))
            .filter_map(hierarchy_item)
            .collect(),
    )
}

fn hierarchy_item(class: &ClassRef<'_>) -> Option<TypeHierarchyItem> {
    let declaration = class.declaration;
    let text = std::fs::read(class.path).ok()?;
    let lines = LineIndex::new(&text);
    let (start_line, start_col) = lines.position(&text, declaration.name_start);
    let (end_line, end_col) = lines.position(&text, declaration.name_end);
    let range = Range::new(
        Position::new(start_line, start_col),
        Position::new(end_line, end_col),
    );
    let kind = match declaration.kind {
        IndexSymbolKind::Interface => SymbolKind::INTERFACE,
        IndexSymbolKind::Enum => SymbolKind::ENUM,
        IndexSymbolKind::Trait => SymbolKind::STRUCT,
        _ => SymbolKind::CLASS,
    };
    Some(TypeHierarchyItem {
        name: declaration
            .fqn
            .rsplit('\\')
            .next()
            .unwrap_or(&declaration.fqn)
            .to_string(),
        kind,
        tags: None,
        detail: Some(declaration.fqn.clone()),
        uri: path_to_uri(class.path),
        range,
        selection_range: range,
        data: Some(json!({"fqn": declaration.fqn})),
    })
}

/// Index locations as LSP locations, with positions read from each target file.
fn to_locations(targets: &[phplumb_index::Location]) -> Vec<Location> {
    targets
        .iter()
        .filter_map(|target| {
            let text = std::fs::read(&target.path).ok()?;
            let lines = LineIndex::new(&text);
            let (start_line, start_col) = lines.position(&text, target.start);
            let (end_line, end_col) = lines.position(&text, target.end);
            Some(Location {
                uri: path_to_uri(&target.path),
                range: Range::new(
                    Position::new(start_line, start_col),
                    Position::new(end_line, end_col),
                ),
            })
        })
        .collect()
}

fn invalid(id: RequestId, err: serde_json::Error) -> Response {
    Response::new_err(id, ErrorCode::InvalidParams as i32, err.to_string())
}

/// Hover markdown for the word under the cursor, read from the file on disk.
fn hover(index: Option<&mut Index>, params: HoverParams) -> Option<Hover> {
    let index = index?;
    let position = params.text_document_position_params;
    let path = uri_to_path(&position.text_document.uri)?;
    let source = std::fs::read(&path).ok()?;
    let lines = LineIndex::new(&source);
    let offset = lines.offset(&source, position.position.line, position.position.character)?;
    let found = index.hover(&path, &source, offset)?;
    let (start_line, start_col) = lines.position(&source, found.start);
    let (end_line, end_col) = lines.position(&source, found.end);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: found.markdown,
        }),
        range: Some(Range::new(
            Position::new(start_line, start_col),
            Position::new(end_line, end_col),
        )),
    })
}
