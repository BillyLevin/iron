use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    io::{
        self,
        BufRead as _,
        BufReader,
    },
    mem,
    ops::Range,
    path::{
        Path,
        PathBuf,
    },
    process::{
        self,
        Child,
        ChildStderr,
        ChildStdin,
        ChildStdout,
        Command,
        Stdio,
    },
    sync::{
        self,
        mpsc::{
            Receiver,
            Sender,
        },
    },
    thread::{
        self,
        JoinHandle,
    },
};

use anyhow::Context as _;
use gen_lsp_types::{
    ClientCapabilities,
    DiagnosticRefreshRequest,
    DidChangeTextDocumentNotification,
    DidChangeTextDocumentParams,
    DidCloseTextDocumentNotification,
    DidCloseTextDocumentParams,
    DidOpenTextDocumentNotification,
    DidOpenTextDocumentParams,
    DidSaveTextDocumentNotification,
    DidSaveTextDocumentParams,
    ErrorCodes,
    GeneralClientCapabilities,
    InitializeParams,
    InitializeRequest,
    InitializeResult,
    InitializedNotification,
    InitializedParams,
    LanguageKind,
    LspNotificationMethod,
    LspRequestMethod,
    PositionEncodingKind,
    PublishDiagnosticsParams,
    Save,
    ServerCapabilities,
    ShowMessageRequest,
    TextDocumentClientCapabilities,
    TextDocumentContentChangeEvent,
    TextDocumentContentChangePartial,
    TextDocumentContentChangeWholeDocument,
    TextDocumentIdentifier,
    TextDocumentItem,
    TextDocumentSync,
    TextDocumentSyncClientCapabilities,
    TextDocumentSyncKind,
    VersionedTextDocumentIdentifier,
    WorkDoneProgressParams,
    WorkspaceClientCapabilities,
    WorkspaceFolder,
    WorkspaceFolders,
    WorkspaceFoldersInitializeParams,
    WorkspaceFoldersRequest,
    json_rpc::{
        Error,
        Id,
        RequestObject,
        ResponseObject,
    },
};
use ropey::Rope;
use serde::{
    Deserialize,
    Serialize,
};
use serde_json::json;
use url::Url;

use crate::{
    document::TextEdit,
    language::Language,
    text::{
        ByteIndex,
        RopeSliceExt as _,
    },
};

/// Used to send/receive messages to/from a language server.
#[derive(Debug)]
pub(crate) struct LspClient {
    worker_tx: Sender<WorkerInput>,
    event_rx: Receiver<LspEvent>,
    worker_handle: Option<JoinHandle<()>>,
}

impl LspClient {
    pub(crate) fn new() -> anyhow::Result<Self> {
        let workspace_root = WorkspaceRoot::new()?;

        let (worker_tx, worker_rx) = sync::mpsc::channel();
        let (event_tx, event_rx) = sync::mpsc::channel();

        let worker_handle = spawn_worker(worker_tx.clone(), worker_rx, event_tx, workspace_root);

        Ok(Self {
            worker_tx,
            event_rx,
            worker_handle: Some(worker_handle),
        })
    }

    pub(crate) fn document_opened(&self, snapshot: DocumentSnapshot) {
        self.send(LspAction::OpenDocument(snapshot));
    }

    pub(crate) fn document_changed(&self, snapshot: DocumentSnapshot, edit: LspTextEdit) {
        self.send(LspAction::ChangeDocument { snapshot, edit });
    }

    pub(crate) fn document_saved(&self, snapshot: DocumentSnapshot) {
        self.send(LspAction::SaveDocument(snapshot));
    }

    pub(crate) fn document_closed(&self, id: DocumentLspId) {
        self.send(LspAction::CloseDocument(id));
    }

    pub(crate) fn events(&self) -> impl Iterator<Item = LspEvent> {
        self.event_rx.try_iter()
    }

    fn send(&self, action: LspAction) {
        if self
            .worker_tx
            .send(WorkerInput::Send(Box::new(action)))
            .is_err()
        {
            log::error!("`worker_rx` is unexpectedly dead");
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.worker_tx.send(WorkerInput::Shutdown);
        if self
            .worker_handle
            .take()
            .is_some_and(|handle| handle.join().is_err())
        {
            log::error!("lsp worker did not shutdown gracefully");
        }
    }
}

/// The LSP workspace root. For now, this is simply the CWD of the process. We
/// will only support LSP features for files descending from this root.
#[derive(Debug)]
struct WorkspaceRoot {
    path: CanonicalPath,
    folder: WorkspaceFolder,
}

impl WorkspaceRoot {
    fn new() -> anyhow::Result<Self> {
        let canonical_path = CanonicalPath::new(
            env::current_dir()
                .context("failed to find workspace root")?
                .as_path(),
        )?;
        let path = canonical_path.as_path();

        let uri = Url::from_directory_path(path)
            .map_err(|()| anyhow::anyhow!("failed to construct workspace root URL"))?;

        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();

        Ok(Self {
            path: canonical_path,
            folder: WorkspaceFolder { uri, name },
        })
    }

    fn contains(&self, path: &CanonicalPath) -> bool {
        path.as_path().starts_with(self.path.as_path())
    }

    const fn url(&self) -> &Url {
        &self.folder.uri
    }

    fn as_path(&self) -> &Path {
        self.path.as_path()
    }

    fn folder(&self) -> WorkspaceFolder {
        self.folder.clone()
    }
}

#[derive(Debug)]
pub(crate) struct DocumentSnapshot {
    id: DocumentLspId,
    text: Rope,
    version: DocumentVersion,
}

impl DocumentSnapshot {
    pub(crate) const fn new(id: DocumentLspId, text: Rope, version: DocumentVersion) -> Self {
        Self { id, text, version }
    }
}

#[derive(Debug, Clone, Copy, derive_more::Into)]
pub(crate) struct DocumentVersion(i32);

impl DocumentVersion {
    pub(crate) const fn value(self) -> i32 {
        self.0
    }

    pub(crate) const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl Default for DocumentVersion {
    fn default() -> Self {
        Self(1)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DocumentLspId {
    path: CanonicalPath,
    url: Url,
    language: Language,
}

impl DocumentLspId {
    pub(crate) fn new(path: &Path, language: Language) -> anyhow::Result<Self> {
        let path = CanonicalPath::new(path)?;
        let url = path.to_file_url()?;

        Ok(Self {
            path,
            url,
            language,
        })
    }

    pub(crate) const fn url(&self) -> &Url {
        &self.url
    }
}

#[derive(Debug)]
struct LspWorker {
    worker_tx: Sender<WorkerInput>,
    worker_rx: Receiver<WorkerInput>,
    event_tx: Sender<LspEvent>,
    servers: LanguageServers,
    workspace_root: WorkspaceRoot,
}

impl LspWorker {
    fn new(
        worker_tx: Sender<WorkerInput>,
        worker_rx: Receiver<WorkerInput>,
        event_tx: Sender<LspEvent>,
        workspace_root: WorkspaceRoot,
    ) -> Self {
        Self {
            worker_tx,
            worker_rx,
            event_tx,
            servers: LanguageServers::new(),
            workspace_root,
        }
    }

    fn run(mut self) {
        while let Ok(input) = self.worker_rx.recv() {
            match input {
                WorkerInput::Send(action) => self.handle_action(*action),
                WorkerInput::Receive { message, server } => {
                    if let Err(error) = self.receive(server, message) {
                        self.handle_server_error(server, error);
                    }
                }
                WorkerInput::KillServer { server, error } => {
                    self.handle_server_error(server, ServerError::Fatal(error));
                }
                WorkerInput::Shutdown => {
                    break;
                }
            }
        }
    }

    fn handle_action(&mut self, action: LspAction) {
        let Some(id) = LanguageServerId::try_from_action(&action, &self.workspace_root) else {
            return;
        };

        let Some(server) = self.get_or_spawn_server(id, self.worker_tx.clone()) else {
            return;
        };

        match server.send_or_enqueue(action) {
            Ok(()) => {}
            Err(error) => self.handle_server_error(id, error),
        }
    }

    fn get_or_spawn_server(
        &mut self,
        id: LanguageServerId,
        worker_tx: Sender<WorkerInput>,
    ) -> Option<&mut LanguageServer> {
        self.servers
            .map
            .entry(id)
            .or_insert_with_key(|key| {
                match LanguageServer::new(worker_tx, *key, &self.workspace_root) {
                    Ok(server) => LanguageServerEntry::Active(server),
                    Err(error) => {
                        log::error!("failed to start language server: {error:#}");
                        LanguageServerEntry::Failed
                    }
                }
            })
            .server_mut()
    }

    fn receive(
        &mut self,
        server_id: LanguageServerId,
        message: ServerMessage,
    ) -> Result<(), ServerError> {
        match message {
            ServerMessage::RequestOrNotification(message) => {
                match message.id() {
                    Some(_) => self.handle_server_request(server_id, &message),
                    None => self.handle_server_notification(server_id, &message),
                }
            }
            ServerMessage::Response(response) => self.handle_server_response(server_id, &response),
        }
    }

    fn handle_server_request(
        &mut self,
        server_id: LanguageServerId,
        request: &RequestObject,
    ) -> Result<(), ServerError> {
        let Some(server) = self.servers.try_get_mut(server_id)? else {
            return Ok(());
        };

        server.handle_request(request, &self.workspace_root)
    }

    fn handle_server_response(
        &mut self,
        server_id: LanguageServerId,
        response: &ResponseObject,
    ) -> Result<(), ServerError> {
        let Some(server) = self.servers.try_get_mut(server_id)? else {
            return Ok(());
        };

        server.handle_response(response)
    }

    fn handle_server_notification(
        &mut self,
        server_id: LanguageServerId,
        notification: &RequestObject,
    ) -> Result<(), ServerError> {
        let Some(server) = self.servers.try_get_mut(server_id)? else {
            return Ok(());
        };

        if let Some(event) = server.handle_notification(notification)? {
            self.event_tx
                .send(event)
                .context("`event_rx` is unexpectedly dead")
                .map_err(ServerError::Fatal)?;
        }

        Ok(())
    }

    fn handle_server_error(&mut self, server_id: LanguageServerId, error: ServerError) {
        match error {
            ServerError::Fatal(error) => {
                self.servers.kill_server(server_id, &error);
            }
            ServerError::NonFatal(error) => {
                log::warn!("non-fatal LSP error: {error:#}");
            }
        }
    }
}

#[derive(Debug)]
struct LanguageServers {
    map: HashMap<LanguageServerId, LanguageServerEntry>,
}

impl LanguageServers {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Attempts to get the underlying [`LanguageServer`] for the given `id`, if
    /// it's alive.
    fn try_get_mut(
        &mut self,
        id: LanguageServerId,
    ) -> Result<Option<&mut LanguageServer>, ServerError> {
        Ok(self
            .map
            .get_mut(&id)
            .context("no corresponding server found")
            .map_err(ServerError::NonFatal)?
            .server_mut())
    }

    fn kill_server(&mut self, id: LanguageServerId, error: &anyhow::Error) {
        match self.map.insert(id, LanguageServerEntry::Failed) {
            Some(LanguageServerEntry::Active(_)) => {
                log::error!("fatal LSP error. killing server: {error:#}");
            }
            Some(LanguageServerEntry::Failed) | None => {}
        }
    }
}

#[derive(Debug)]
enum LanguageServerEntry {
    Active(LanguageServer),
    Failed,
}

impl LanguageServerEntry {
    const fn server_mut(&mut self) -> Option<&mut LanguageServer> {
        match *self {
            Self::Active(ref mut server) => Some(server),
            Self::Failed => None,
        }
    }
}

#[derive(Debug)]
struct LanguageServer {
    state: ServerState,
    writer_tx: Option<Sender<serde_json::Value>>,
    request_ids: RequestIdGenerator,
    /// Requests that are expecting a response from the language server.
    #[expect(clippy::zero_sized_map_values, reason = "won't be zero-sized for long")]
    pending: HashMap<RequestId, PendingRequest>,
    server_process: LanguageServerProcess,
    writer_handle: Option<JoinHandle<()>>,
    reader_handle: Option<JoinHandle<()>>,
    error_logger_handle: Option<JoinHandle<()>>,
}

impl LanguageServer {
    fn new(
        worker_tx: Sender<WorkerInput>,
        id: LanguageServerId,
        workspace_root: &WorkspaceRoot,
    ) -> anyhow::Result<Self> {
        let mut server = LanguageServerProcess {
            process: Command::new(id.lsp_command())
                .current_dir(workspace_root.as_path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("failed to spawn language server")?,
        };

        let (writer_tx, writer_rx) = sync::mpsc::channel::<serde_json::Value>();

        let writer_handle = spawn_writer(
            server
                .process
                .stdin
                .take()
                .context("failed to take stdin")?,
            id,
            writer_rx,
            worker_tx.clone(),
        );
        let reader_handle = spawn_reader(
            server
                .process
                .stdout
                .take()
                .context("failed to take stdout")?,
            id,
            worker_tx,
        );
        let error_logger_handle = spawn_error_logger(
            server
                .process
                .stderr
                .take()
                .context("failed to take stderr")?,
        );

        let mut this = Self {
            state: ServerState::Initializing { queue: Vec::new() },
            writer_tx: Some(writer_tx),
            request_ids: RequestIdGenerator::new(),
            #[expect(clippy::zero_sized_map_values, reason = "won't be zero-sized for long")]
            pending: HashMap::new(),
            server_process: server,
            writer_handle: Some(writer_handle),
            reader_handle: Some(reader_handle),
            error_logger_handle: Some(error_logger_handle),
        };
        this.send_initialize(workspace_root)
            .map_err(ServerError::inner)?;

        Ok(this)
    }

    fn handle_response(&mut self, response: &ResponseObject) -> Result<(), ServerError> {
        let request_id = RequestId::from_jsonrpc_id(response.id())
            .context("received server response with unsupported request ID")
            .map_err(ServerError::NonFatal)?;

        let matched_request = self
            .pending
            .remove(&request_id)
            .context("received server response with no corresponding request")
            .map_err(ServerError::NonFatal)?;

        match matched_request {
            PendingRequest::Initialize => self.handle_initialized(response),
        }
    }

    fn handle_notification(
        &self,
        notification: &RequestObject,
    ) -> Result<Option<LspEvent>, ServerError> {
        #[expect(clippy::wildcard_enum_match_arm, reason = "it's fine here")]
        match LspNotificationMethod::from(notification.method()) {
            LspNotificationMethod::TextDocumentPublishDiagnostics => {
                let params = PublishDiagnosticsParams::deserialize(
                    notification
                        .params()
                        .context("publish diagnostics params are missing")
                        .map_err(ServerError::NonFatal)?,
                )
                .context("failed to parse publish diagnostic params")
                .map_err(ServerError::NonFatal)?;

                let Some(capabilities) = self.capabilities() else {
                    return Ok(None);
                };

                Ok(Some(LspEvent::PublishDiagnostics {
                    params,
                    position_encoding: capabilities.position_encoding,
                }))
            }
            method => {
                log::info!("received unsupported notification: {method}");
                Ok(None)
            }
        }
    }

    fn handle_request(
        &self,
        request: &RequestObject,
        workspace_root: &WorkspaceRoot,
    ) -> Result<(), ServerError> {
        let request_id = request
            .id()
            .cloned()
            .context("request ID missing")
            .map_err(ServerError::NonFatal)?;

        #[expect(clippy::wildcard_enum_match_arm, reason = "it's fine here")]
        match LspRequestMethod::from(request.method()) {
            LspRequestMethod::WindowShowMessageRequest => {
                self.send_response::<ShowMessageRequest>(request_id, None)
            }
            LspRequestMethod::WorkspaceDiagnosticRefresh => {
                self.send_response::<DiagnosticRefreshRequest>(request_id, ())
            }
            LspRequestMethod::WorkspaceWorkspaceFolders => {
                self.send_response::<WorkspaceFoldersRequest>(
                    request_id,
                    Some(vec![workspace_root.folder()]),
                )
            }
            method => {
                log::info!("received unsupported request: {method}");

                self.send_error(
                    request_id,
                    ErrorCodes::MethodNotFound,
                    "request not supported",
                )
            }
        }
    }

    const fn capabilities(&self) -> Option<Capabilities> {
        match self.state {
            ServerState::Ready { capabilities } => Some(capabilities),
            ServerState::Initializing { .. } => None,
        }
    }

    fn handle_initialized(&mut self, response: &ResponseObject) -> Result<(), ServerError> {
        if let Some(error) = response.error() {
            return Err(ServerError::Fatal(anyhow::anyhow!(
                "language server initialization failed: {error:?}"
            )));
        }

        let result: InitializeResult = response
            .result()
            .context("initialize response did not contain a result")
            .and_then(|value| {
                InitializeResult::deserialize(value)
                    .context("failed to deserialize initialize result")
            })
            .map_err(ServerError::Fatal)?;

        let capabilities = Capabilities::new(result.capabilities).map_err(ServerError::Fatal)?;

        let queue = match self.state {
            ServerState::Initializing { ref mut queue } => mem::take(queue),
            ServerState::Ready { .. } => {
                return Err(ServerError::Fatal(anyhow::anyhow!(
                    "received initialization response in invalid server state"
                )));
            }
        };

        self.send_notification::<InitializedNotification>(InitializedParams {})
            .map_err(|error| {
                match error {
                    ServerError::Fatal(error) | ServerError::NonFatal(error) => {
                        ServerError::Fatal(error)
                    }
                }
            })?;

        self.state = ServerState::Ready { capabilities };

        for action in queue {
            self.send_action(action, capabilities)
                .map_err(ServerError::Fatal)?;
        }

        Ok(())
    }

    fn send_initialize(&mut self, workspace_root: &WorkspaceRoot) -> Result<(), ServerError> {
        self.send_request::<InitializeRequest>(PendingRequest::Initialize, &InitializeParams {
            process_id: process::id().try_into().ok(),
            client_info: None,
            locale: None,
            #[expect(
                deprecated,
                reason = "explicitly setting the deprecated field to `None`"
            )]
            root_path: None,
            #[expect(
                deprecated,
                reason = "while `root_uri` is deprecated, it's still worth sending for now for \
                          compatibility with older servers"
            )]
            root_uri: Some(workspace_root.url().clone()),
            capabilities: ClientCapabilities {
                workspace: Some(WorkspaceClientCapabilities {
                    workspace_folders: Some(true),
                    ..Default::default()
                }),
                text_document: Some(TextDocumentClientCapabilities {
                    synchronization: Some({
                        TextDocumentSyncClientCapabilities {
                            did_save: Some(true),
                            dynamic_registration: None,
                            will_save: None,
                            will_save_wait_until: None,
                        }
                    }),
                    ..Default::default()
                }),
                notebook_document: None,
                window: None,
                general: Some(GeneralClientCapabilities {
                    stale_request_support: None,
                    regular_expressions: None,
                    markdown: None,
                    position_encodings: Some(vec![
                        PositionEncodingKind::UTF8,
                        PositionEncodingKind::UTF32,
                        PositionEncodingKind::UTF16,
                    ]),
                }),
                experimental: None,
            },
            // TODO: we may want some stuff here, e.g. telling rust-analyzer to use
            // nightly fmt or whatever
            initialization_options: None,
            trace: None,
            work_done_progress_params: WorkDoneProgressParams::default(),
            workspace_folders_initialize_params: WorkspaceFoldersInitializeParams {
                workspace_folders: Some(WorkspaceFolders::from(vec![workspace_root.folder()])),
            },
        })
    }

    fn send_or_enqueue(&mut self, action: LspAction) -> Result<(), ServerError> {
        match self.state {
            ServerState::Initializing { ref mut queue } => {
                queue.push(action);
                Ok(())
            }
            ServerState::Ready { capabilities } => {
                self.send_action(action, capabilities)
                    .map_err(ServerError::Fatal)
            }
        }
    }

    fn send_action(&self, action: LspAction, capabilities: Capabilities) -> anyhow::Result<()> {
        match action {
            LspAction::OpenDocument(snapshot) => {
                if capabilities.text_document_sync.open_close {
                    self.send_notification::<DidOpenTextDocumentNotification>(
                        DidOpenTextDocumentParams {
                            text_document: TextDocumentItem {
                                language_id: LanguageKind::from(snapshot.id.language),
                                uri: snapshot.id.url,
                                text: snapshot.text.to_string(),
                                version: i32::from(snapshot.version),
                            },
                        },
                    )
                    .map_err(ServerError::inner)?;
                }
            }
            LspAction::ChangeDocument { snapshot, edit } => {
                let changes = match capabilities.text_document_sync.change {
                    TextDocumentSyncKind::None => return Ok(()),
                    TextDocumentSyncKind::Full => {
                        vec![
                            TextDocumentContentChangeWholeDocument::new(snapshot.text.to_string())
                                .into(),
                        ]
                    }
                    TextDocumentSyncKind::Incremental => {
                        let (range, replacement) = edit.edit.into_parts();
                        let range = byte_to_lsp_range(
                            range,
                            capabilities.position_encoding,
                            &edit.initial_text,
                        )?;

                        vec![TextDocumentContentChangeEvent::from(
                            TextDocumentContentChangePartial::new(range, None, replacement),
                        )]
                    }
                };

                self.send_notification::<DidChangeTextDocumentNotification>(
                    DidChangeTextDocumentParams {
                        text_document: VersionedTextDocumentIdentifier {
                            version: snapshot.version.value(),
                            text_document_identifier: TextDocumentIdentifier {
                                uri: snapshot.id.url,
                            },
                        },
                        content_changes: changes,
                    },
                )
                .map_err(ServerError::inner)?;
            }
            LspAction::SaveDocument(snapshot) => {
                match capabilities.text_document_sync.save {
                    SaveCapability::Unsupported => {
                        log::info!("textDocument/didSave not supported. not sending notification");
                        return Ok(());
                    }
                    SaveCapability::Supported { include_text } => {
                        self.send_notification::<DidSaveTextDocumentNotification>(
                            DidSaveTextDocumentParams {
                                text_document: TextDocumentIdentifier {
                                    uri: snapshot.id.url,
                                },
                                text: include_text.then(|| snapshot.text.to_string()),
                            },
                        )
                        .map_err(ServerError::inner)?;
                    }
                }
            }
            LspAction::CloseDocument(id) => {
                if capabilities.text_document_sync.open_close {
                    self.send_notification::<DidCloseTextDocumentNotification>(
                        DidCloseTextDocumentParams {
                            text_document: TextDocumentIdentifier { uri: id.url },
                        },
                    )
                    .map_err(ServerError::inner)?;
                }
            }
        }

        Ok(())
    }

    /// <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/#requestMessage>.
    fn send_request<R>(
        &mut self,
        kind: PendingRequest,
        params: &R::Params,
    ) -> Result<(), ServerError>
    where
        R: gen_lsp_types::Request,
    {
        let id = self.request_ids.next();

        if let Some(ref tx) = self.writer_tx {
            tx.send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": R::METHOD,
                "params": params
            }))
            .context("`writer_rx` is unexpectedly dead")
            .map_err(ServerError::Fatal)?;

            self.pending.insert(id, kind);
            Ok(())
        } else {
            Err(ServerError::Fatal(anyhow::anyhow!(
                "no writer channel exists"
            )))
        }
    }

    /// <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/#notificationMessage>.
    fn send_notification<N>(&self, params: N::Params) -> Result<(), ServerError>
    where
        N: gen_lsp_types::Notification,
    {
        let notification = serde_json::to_value(RequestObject::from_notification::<N>(params))
            .context("failed to construct notification object")
            .map_err(ServerError::NonFatal)?;

        self.send(notification)
    }

    /// <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/#responseMessage>.
    fn send_response<R>(&self, id: Id, result: R::Result) -> Result<(), ServerError>
    where
        R: gen_lsp_types::Request,
    {
        let success = serde_json::to_value(ResponseObject::from_success::<R>(id, result))
            .context("failed to construct success object")
            .map_err(ServerError::NonFatal)?;

        self.send(success)
    }

    fn send_error(&self, id: Id, code: ErrorCodes, message: &str) -> Result<(), ServerError> {
        let error = serde_json::to_value(ResponseObject::from_error(id, Error {
            code,
            message: message.to_owned(),
            data: None,
        }))
        .context("failed to construct error object")
        .map_err(ServerError::NonFatal)?;

        self.send(error)
    }

    fn send(&self, message: serde_json::Value) -> Result<(), ServerError> {
        if let Some(ref tx) = self.writer_tx {
            tx.send(message)
                .context("`writer_rx` is unexpectedly dead")
                .map_err(ServerError::Fatal)
        } else {
            Err(ServerError::Fatal(anyhow::anyhow!(
                "no writer channel exists"
            )))
        }
    }

    fn shutdown(&mut self) {
        self.writer_tx = None;
        self.server_process.shutdown();
        for (name, handle) in [
            ("writer", &mut self.writer_handle),
            ("reader", &mut self.reader_handle),
            ("error logger", &mut self.error_logger_handle),
        ] {
            if handle.take().is_some_and(|handle| handle.join().is_err()) {
                log::error!("language server {name} thread panicked");
            }
        }
    }
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug)]
struct LanguageServerProcess {
    process: Child,
}

impl LanguageServerProcess {
    fn shutdown(&mut self) {
        if let Err(error) = self.process.kill()
            && error.kind() != io::ErrorKind::InvalidInput
        {
            log::warn!("failed to kill language server: {error}");
        }
        if let Err(error) = self.process.wait() {
            log::warn!("failed to wait for language server: {error}");
        }
    }
}

impl Drop for LanguageServerProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LanguageServerId {
    RustAnalyzer,
}

impl LanguageServerId {
    fn try_from_action(action: &LspAction, workspace_root: &WorkspaceRoot) -> Option<Self> {
        let id = match *action {
            LspAction::OpenDocument(ref snapshot)
            | LspAction::ChangeDocument { ref snapshot, .. }
            | LspAction::SaveDocument(ref snapshot) => &snapshot.id,
            LspAction::CloseDocument(ref id) => id,
        };

        // we only support files in the workspace root
        if !workspace_root.contains(&id.path) {
            return None;
        }

        Self::from_language(id.language)
    }

    const fn from_language(language: Language) -> Option<Self> {
        match language {
            Language::Rust => Some(Self::RustAnalyzer),
            Language::Toml | Language::Text => None,
        }
    }

    const fn lsp_command(self) -> &'static str {
        match self {
            Self::RustAnalyzer => "rust-analyzer",
        }
    }
}

impl From<Language> for LanguageKind {
    fn from(language: Language) -> Self {
        match language {
            Language::Rust => Self::Rust,
            Language::Toml => Self::Custom(Cow::Borrowed("toml")),
            Language::Text => Self::Plaintext,
        }
    }
}

#[derive(Debug)]
enum ServerState {
    Initializing { queue: Vec<LspAction> },
    Ready { capabilities: Capabilities },
}

#[derive(Debug)]
enum ServerError {
    /// A fatal error is one that requires us to shut down the language server.
    Fatal(anyhow::Error),
    /// A non-fatal error is one that allows us to keep the language server
    /// alive in spite of the error.
    NonFatal(anyhow::Error),
}

impl ServerError {
    fn inner(self) -> anyhow::Error {
        match self {
            Self::Fatal(error) | Self::NonFatal(error) => error,
        }
    }
}

/// The normalized capabilities that the server has reported that it supports.
/// We only store the the ones we care about.
#[derive(Debug, Clone, Copy)]
struct Capabilities {
    position_encoding: PositionEncoding,
    text_document_sync: TextDocumentSyncCapabilities,
}

impl Capabilities {
    fn new(capabilities: ServerCapabilities) -> anyhow::Result<Self> {
        let ServerCapabilities {
            position_encoding,
            text_document_sync,
            notebook_document_sync: _notebook_document_sync,
            completion_provider: _completion_provider,
            hover_provider: _hover_provider,
            signature_help_provider: _signature_help_provider,
            declaration_provider: _declaration_provider,
            definition_provider: _definition_provider,
            type_definition_provider: _type_definition_provider,
            implementation_provider: _implementation_provider,
            references_provider: _references_provider,
            document_highlight_provider: _document_highlight_provider,
            document_symbol_provider: _document_symbol_provider,
            code_action_provider: _code_action_provider,
            code_lens_provider: _code_lens_provider,
            document_link_provider: _document_link_provider,
            color_provider: _color_provider,
            workspace_symbol_provider: _workspace_symbol_provider,
            document_formatting_provider: _document_formatting_provider,
            document_range_formatting_provider: _document_range_formatting_provider,
            document_on_type_formatting_provider: _document_on_type_formatting_provider,
            rename_provider: _rename_provider,
            folding_range_provider: _folding_range_provider,
            selection_range_provider: _selection_range_provider,
            execute_command_provider: _execute_command_provider,
            call_hierarchy_provider: _call_hierarchy_provider,
            linked_editing_range_provider: _linked_editing_range_provider,
            semantic_tokens_provider: _semantic_tokens_provider,
            moniker_provider: _moniker_provider,
            type_hierarchy_provider: _type_hierarchy_provider,
            inline_value_provider: _inline_value_provider,
            inlay_hint_provider: _inlay_hint_provider,
            diagnostic_provider: _diagnostic_provider,
            inline_completion_provider: _inline_completion_provider,
            workspace: _workspace,
            experimental: _experimental,
        } = capabilities;

        Ok(Self {
            position_encoding: PositionEncoding::try_from(
                position_encoding.unwrap_or(PositionEncodingKind::UTF16),
            )?,
            text_document_sync: match text_document_sync {
                Some(TextDocumentSync::Kind(
                    kind @ (TextDocumentSyncKind::Incremental | TextDocumentSyncKind::Full),
                )) => {
                    TextDocumentSyncCapabilities {
                        change: kind,
                        open_close: true,
                        save: SaveCapability::Supported {
                            include_text: false,
                        },
                    }
                }
                Some(TextDocumentSync::Kind(kind @ TextDocumentSyncKind::None)) => {
                    TextDocumentSyncCapabilities {
                        change: kind,
                        open_close: false,
                        save: SaveCapability::Unsupported,
                    }
                }
                Some(TextDocumentSync::Options(options)) => {
                    TextDocumentSyncCapabilities {
                        open_close: options.open_close.unwrap_or(false),
                        change: options.change.unwrap_or(TextDocumentSyncKind::None),
                        save: match options.save {
                            Some(save_settings) => {
                                match save_settings {
                                    Save::Bool(true) => {
                                        SaveCapability::Supported {
                                            include_text: false,
                                        }
                                    }
                                    Save::Bool(false) => SaveCapability::Unsupported,
                                    Save::SaveOptions(save_options) => {
                                        SaveCapability::Supported {
                                            include_text: save_options
                                                .include_text
                                                .unwrap_or(false),
                                        }
                                    }
                                }
                            }
                            None => SaveCapability::Unsupported,
                        },
                    }
                }
                None => TextDocumentSyncCapabilities::default(),
            },
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct TextDocumentSyncCapabilities {
    change: TextDocumentSyncKind,
    open_close: bool,
    save: SaveCapability,
}

impl Default for TextDocumentSyncCapabilities {
    fn default() -> Self {
        Self {
            change: TextDocumentSyncKind::None,
            open_close: false,
            save: SaveCapability::Unsupported,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SaveCapability {
    Unsupported,
    Supported { include_text: bool },
}

#[derive(Debug)]
struct RequestIdGenerator {
    current: RequestId,
}

impl RequestIdGenerator {
    const fn new() -> Self {
        Self {
            current: RequestId(1),
        }
    }

    const fn next(&mut self) -> RequestId {
        let result = self.current;
        self.current.0 += 1_i32;
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
struct RequestId(i32);

impl RequestId {
    fn from_jsonrpc_id(id: &Id) -> Option<Self> {
        match *id {
            Id::Number(num) => i32::try_from(num).map(Self).ok(),
            Id::String(_) | Id::Null => None,
        }
    }
}

#[derive(Debug)]
enum PendingRequest {
    Initialize,
}

#[derive(Debug)]
enum WorkerInput {
    Send(Box<LspAction>),
    Receive {
        server: LanguageServerId,
        message: ServerMessage,
    },
    KillServer {
        server: LanguageServerId,
        error: anyhow::Error,
    },
    Shutdown,
}

#[derive(Debug)]
#[expect(
    clippy::enum_variant_names,
    reason = "probably won't all have same suffix forever"
)]
enum LspAction {
    OpenDocument(DocumentSnapshot),
    ChangeDocument {
        snapshot: DocumentSnapshot,
        edit: LspTextEdit,
    },
    SaveDocument(DocumentSnapshot),
    CloseDocument(DocumentLspId),
}

#[derive(Debug)]
pub(crate) enum LspEvent {
    PublishDiagnostics {
        params: PublishDiagnosticsParams,
        position_encoding: PositionEncoding,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ServerMessage {
    Response(ResponseObject),
    RequestOrNotification(RequestObject),
}

#[derive(Debug)]
pub(crate) struct LspTextEdit {
    initial_text: Rope,
    edit: TextEdit,
}

impl LspTextEdit {
    pub(crate) const fn new(initial_text: Rope, edit: TextEdit) -> Self {
        Self { initial_text, edit }
    }
}

#[derive(Debug, Clone)]
struct CanonicalPath {
    path: PathBuf,
}

impl CanonicalPath {
    fn new(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            path: path.canonicalize().context("failed to canonicalize path")?,
        })
    }

    fn as_path(&self) -> &Path {
        &self.path
    }

    fn to_file_url(&self) -> anyhow::Result<Url> {
        Url::from_file_path(&self.path)
            .map_err(|()| anyhow::anyhow!("failed to file URL from path"))
    }
}

#[derive(Debug, Clone, Copy)]
/// A more restrictive version of [`PositionEncodingKind`] that doesn't allow
/// for custom encoding.
pub(crate) enum PositionEncoding {
    /// Character offsets count UTF-8 code units (e.g. bytes).
    UTF8,
    /// Character offsets count UTF-16 code units.
    ///
    /// This is the default and must always be supported
    /// by servers.
    UTF16,
    /// Character offsets count UTF-32 code units.
    ///
    /// Implementation note: these are the same as Unicode codepoints,
    /// so this `PositionEncoding` may also be used for an
    /// encoding-agnostic representation of character offsets.
    UTF32,
}

impl TryFrom<PositionEncodingKind> for PositionEncoding {
    type Error = anyhow::Error;

    fn try_from(value: PositionEncodingKind) -> Result<Self, Self::Error> {
        match value {
            PositionEncodingKind::UTF8 => Ok(Self::UTF8),
            PositionEncodingKind::UTF16 => Ok(Self::UTF16),
            PositionEncodingKind::UTF32 => Ok(Self::UTF32),
            PositionEncodingKind::Custom(_) => {
                Err(anyhow::anyhow!("custom position encoding is unsupported"))
            }
        }
    }
}

fn byte_to_lsp_range(
    range: Range<ByteIndex>,
    position_encoding: PositionEncoding,
    text: &Rope,
) -> anyhow::Result<gen_lsp_types::Range> {
    let text = text.slice(..);

    let translate = |index: ByteIndex| -> anyhow::Result<gen_lsp_types::Position> {
        let line_index = text.line_idx_containing_byte(index);
        let line = text.line_at(line_index);
        let byte_offset = index.value() - text.line_start_byte(line_index).value();

        let offset = match position_encoding {
            PositionEncoding::UTF8 => byte_offset,
            PositionEncoding::UTF16 => line.byte_to_utf16_idx(byte_offset),
            PositionEncoding::UTF32 => line.byte_to_char_idx(byte_offset),
        };

        Ok(gen_lsp_types::Position {
            line: u32::try_from(line_index.value()).context("line index is too large")?,
            character: u32::try_from(offset).context("character offset is too large")?,
        })
    };

    Ok(gen_lsp_types::Range {
        start: translate(range.start)?,
        end: translate(range.end)?,
    })
}

fn read_message<Input>(input: &mut Input) -> anyhow::Result<serde_json::Value>
where
    Input: io::BufRead,
{
    let mut buffer = String::new();
    let mut content_length: Option<usize> = None;

    loop {
        buffer.clear();

        anyhow::ensure!(
            input.read_line(&mut buffer)? > 0,
            "reached end of message before it could be parsed"
        );

        if buffer == "\r\n" {
            break;
        }

        match buffer.split_once(": ") {
            Some((key, value)) if key.eq_ignore_ascii_case("Content-Length") => {
                content_length = Some(
                    value
                        .trim()
                        .parse()
                        .context("content length is not a valid `usize`")?,
                );
            }
            Some(_) | None => {}
        }
    }

    let mut content = vec![0_u8; content_length.context("no content length found")?];
    input
        .read_exact(&mut content)
        .context("failed to read content")?;

    serde_json::from_slice(&content).context("failed to convert content to json")
}

fn write_message<Output>(output: &mut Output, message: &serde_json::Value) -> anyhow::Result<()>
where
    Output: io::Write,
{
    let message_bytes = serde_json::to_vec(message)?;

    output
        .write_all(format!("Content-Length: {}\r\n\r\n", message_bytes.len()).as_bytes())
        .context("failed to write header")?;

    output
        .write_all(&message_bytes)
        .context("failed to write body")?;

    output.flush().context("failed to flush")
}

fn spawn_reader(
    stdout: ChildStdout,
    server: LanguageServerId,
    worker_tx: Sender<WorkerInput>,
) -> JoinHandle<()> {
    let mut stdout = BufReader::new(stdout);

    thread::spawn(move || {
        loop {
            match read_message(&mut stdout).and_then(|message| {
                serde_json::from_value::<ServerMessage>(message)
                    .context("failed to parse LSP response")
            }) {
                Ok(message) => {
                    if worker_tx
                        .send(WorkerInput::Receive { server, message })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    // nothing to do here if the send fails. means the receiver is
                    // already dropped so everything has been shutdown.
                    let _ = worker_tx.send(WorkerInput::KillServer {
                        server,
                        error: error.context("failed to read LSP response message"),
                    });
                    break;
                }
            }
        }
    })
}

fn spawn_writer(
    mut stdin: ChildStdin,
    server: LanguageServerId,
    writer_rx: Receiver<serde_json::Value>,
    worker_tx: Sender<WorkerInput>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        for message in writer_rx {
            if let Err(error) = write_message(&mut stdin, &message) {
                // nothing to do here if the send fails. means the receiver is
                // already dropped so everything has been shutdown.
                let _ = worker_tx.send(WorkerInput::KillServer {
                    server,
                    error: error.context("failed to write LSP request/notification message"),
                });
                break;
            }
        }
    })
}

fn spawn_error_logger(stderr: ChildStderr) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => log::error!("{line}"),
                Err(error) => {
                    log::error!("stderr logger died - not killing LSP: {error:#}");
                    break;
                }
            }
        }
    })
}

fn spawn_worker(
    worker_tx: Sender<WorkerInput>,
    worker_rx: Receiver<WorkerInput>,
    event_tx: Sender<LspEvent>,
    workspace_root: WorkspaceRoot,
) -> JoinHandle<()> {
    thread::spawn(move || LspWorker::new(worker_tx, worker_rx, event_tx, workspace_root).run())
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::json;

    use super::*;

    fn message(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    fn message_with_headers(headers: &str, body: &str) -> Vec<u8> {
        format!("{headers}\r\n{body}").into_bytes()
    }

    #[test]
    fn read_message_reads_json_body() {
        let mut input = io::Cursor::new(message(r#"{"jsonrpc":"2.0","id":1}"#));

        assert_eq!(
            read_message(&mut input).unwrap(),
            json!({"jsonrpc": "2.0", "id": 1_i32})
        );
    }

    #[test]
    fn read_message_requires_content_length() {
        let mut input = io::Cursor::new(b"\r\n{}".as_slice());
        assert!(read_message(&mut input).is_err());
    }

    #[test]
    fn read_message_ignores_content_type() {
        let body = r#"{"method":"initialized"}"#;
        let mut input = io::Cursor::new(message_with_headers(
            &format!(
                "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n",
                body.len()
            ),
            body,
        ));

        assert_eq!(
            read_message(&mut input).unwrap(),
            json!({"method": "initialized"})
        );
    }

    #[test]
    fn read_message_accepts_case_insensitive_headers() {
        let body = r#"{"method":"initialized"}"#;
        let mut input = io::Cursor::new(message_with_headers(
            &format!("content-length: {}\r\n", body.len()),
            body,
        ));

        assert_eq!(
            read_message(&mut input).unwrap(),
            json!({"method": "initialized"})
        );
    }

    #[test]
    fn read_message_reads_one_message_at_a_time() {
        let first = message(r#"{"id":1}"#);
        let second = message(r#"{"id":2}"#);
        let mut input = io::Cursor::new([first, second].concat());

        assert_eq!(read_message(&mut input).unwrap(), json!({"id": 1_i32}));
        assert_eq!(read_message(&mut input).unwrap(), json!({"id": 2_i32}));
    }

    #[test]
    fn read_message_rejects_invalid_json() {
        let mut input = io::Cursor::new(message("not json"));
        assert!(read_message(&mut input).is_err());
    }

    #[test]
    fn write_message_writes_content_length_and_body() {
        let mut output = Vec::new();

        write_message(&mut output, &json!({"id": 1_i32})).unwrap();

        assert_eq!(output, b"Content-Length: 8\r\n\r\n{\"id\":1}".as_slice());
    }
}
