pub(crate) mod diagnostics;

use indexmap::IndexMap;
use lsp_server::{Connection, ErrorCode, Message, RequestId, Response};
use lsp_types::notification::*;
use lsp_types::*;

use error_snippet::IntoDiagnostic;
use lume_driver::CheckedPackageGraph;
use lume_errors::DiagCtx;
use lume_errors::Result;
use lume_span::FileName;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::RwLock;

use crate::backend::diagnostics::MapError;

#[expect(dead_code)]
pub struct Backend {
    /// Defines the parameters retrieved from the language client.
    ///
    /// While not currently used, they will be used for language server
    /// configuration in the future.
    pub params: InitializeParams,

    /// Defines the URL to the workspace root, where the Arcfile should be located.
    workspace_root: Option<Uri>,
    checked_graph: Option<CheckedPackageGraph>,

    /// Lists a mapping between a source URL and their content.
    sources: HashMap<Uri, String>,

    error_files_prev: RwLock<HashSet<Uri>>,
    error_files_curr: RwLock<HashSet<Uri>>,

    dcx: DiagCtx,
}

impl Backend {
    /// Initializes a new [`Backend`] instance with the given parameters
    /// from the client.
    pub fn initialize(params: InitializeParams) -> Self {
        let workspace_folder = params.workspace_folders.as_ref().and_then(|folders| folders.first());

        // Ensure the workspace root has a trailing slash.
        let workspace_root = workspace_folder.map(|folder| {
            if folder.uri.path().as_str().ends_with('/') {
                folder.uri.clone()
            } else {
                let uri = folder.uri.as_str();

                Uri::from_str(&format!("{uri}/")).unwrap()
            }
        });

        Self {
            params,
            workspace_root,
            checked_graph: None,
            sources: HashMap::new(),
            error_files_prev: RwLock::new(HashSet::new()),
            error_files_curr: RwLock::new(HashSet::new()),
            dcx: DiagCtx::new(),
        }
    }

    /// Starts listening on the given [`Connection`] for LSP requests and notifications.
    pub fn listen(&mut self, conn: &Connection) -> Result<()> {
        for msg in &conn.receiver {
            match msg {
                Message::Request(req) => {
                    if conn.handle_shutdown(&req).map_error()? {
                        log::info!("received shutdown request");
                        break;
                    }

                    if let Err(err) = self.handle_request(conn, &req) {
                        log::error!("request {} failed: {err}", &req.method);
                    }
                }
                Message::Notification(req) => {
                    if let Err(err) = self.handle_notification(conn, &req) {
                        log::error!("notification {} failed: {err}", &req.method);
                    }
                }
                Message::Response(resp) => log::error!("got unexpected response: {resp:?}"),
            }
        }

        Ok(())
    }

    #[expect(clippy::unused_self)]
    fn handle_request(&self, conn: &Connection, request: &lsp_server::Request) -> Result<()> {
        Self::err(conn, request.id.clone(), ErrorCode::MethodNotFound, "unhandled method")
    }

    fn handle_notification(&mut self, conn: &Connection, notification: &lsp_server::Notification) -> Result<()> {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let params: DidOpenTextDocumentParams = match serde_json::from_value(notification.params.clone()) {
                    Ok(params) => params,
                    Err(err) => return Err(err.into_diagnostic()),
                };

                log::info!("added document {}", params.text_document.uri.as_str());

                self.sources.insert(params.text_document.uri, params.text_document.text);
                self.check_package_root(conn);
            }
            DidCloseTextDocument::METHOD => {
                let params: DidCloseTextDocumentParams = match serde_json::from_value(notification.params.clone()) {
                    Ok(params) => params,
                    Err(err) => return Err(err.into_diagnostic()),
                };

                log::info!("removed document {}", params.text_document.uri.as_str());

                self.sources.remove(&params.text_document.uri);
                self.check_package_root(conn);
            }
            DidSaveTextDocument::METHOD => {
                let params: DidSaveTextDocumentParams = match serde_json::from_value(notification.params.clone()) {
                    Ok(params) => params,
                    Err(err) => return Err(err.into_diagnostic()),
                };

                log::info!("updated document {} (via save)", params.text_document.uri.as_str());

                self.sources.insert(params.text_document.uri, params.text.unwrap());
                self.check_package_root(conn);
            }
            DidChangeTextDocument::METHOD => {
                let params: DidChangeTextDocumentParams = match serde_json::from_value(notification.params.clone()) {
                    Ok(params) => params,
                    Err(err) => return Err(err.into_diagnostic()),
                };

                log::info!("updated document {} (via change)", params.text_document.uri.as_str());

                let uri = params.text_document.uri;
                let source = params.content_changes.first().unwrap().text.clone();

                self.sources.insert(uri, source);
                self.check_package_root(conn);
            }
            _ => {}
        }

        Ok(())
    }

    /// Runs the Lume driver on the current workspace.
    fn check_package_root(&mut self, conn: &Connection) {
        let Some(root) = &self.workspace_root else {
            panic!("Lume packages without a root Arcfile are not currently supported");
        };

        std::mem::take(&mut self.error_files_prev);
        std::mem::swap(&mut self.error_files_prev, &mut self.error_files_curr);

        let path = PathBuf::from(root.as_str());

        self.checked_graph = self.dcx.with_opt(|dcx| {
            let driver = lume_driver::Driver::from_root(&path, dcx)?;
            let source_overrides = self.build_source_overrides();

            driver.check(lume_session::Options {
                source_overrides: Some(source_overrides),
                ..Default::default()
            })
        });

        self.drain_dcx_diagnostics(conn);
    }

    /// Builds the overrides of source files which we currently have in-memory
    /// in the language server.
    ///
    /// Some of these might not need to be overwritten, as they are the same as
    /// they are on the disk. But, since the operation is a [`IndexMap::extend`]-call,
    /// it's a relatively quick operation.
    fn build_source_overrides(&self) -> IndexMap<FileName, String> {
        let mut source_overrides = IndexMap::new();

        for (url, source) in &self.sources {
            let file_path = PathBuf::from(url.path().as_str());
            let workspace_root = self.workspace_root.as_ref().unwrap().path().as_str();

            let relative_path = if file_path.starts_with(workspace_root) {
                FileName::Real(file_path.strip_prefix(workspace_root).unwrap().to_path_buf())
            } else {
                FileName::Real(file_path)
            };

            source_overrides.insert(relative_path, source.clone());
        }

        source_overrides
    }

    #[expect(dead_code)]
    fn ok<T: serde::Serialize>(conn: &Connection, id: RequestId, message: &T) -> Result<()> {
        let value = match serde_json::to_value(message) {
            Ok(val) => val,
            Err(err) => return Err(err.into_diagnostic()),
        };

        let resp = Response::new_ok(id, value);

        match conn.sender.send(Message::Response(resp)) {
            Ok(()) => Ok(()),
            Err(err) => Err(err.into_diagnostic()),
        }
    }

    fn err(conn: &Connection, id: RequestId, code: ErrorCode, message: &str) -> Result<()> {
        let resp = Response::new_err(id, code as i32, message.into());

        match conn.sender.send(Message::Response(resp)) {
            Ok(()) => Ok(()),
            Err(err) => Err(err.into_diagnostic()),
        }
    }
}
