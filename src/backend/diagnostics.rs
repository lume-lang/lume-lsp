use std::fmt::Write as _;
use std::path::PathBuf;
use std::str::FromStr;

use error_snippet::IntoDiagnostic;
use lsp_server::{Connection, Message};
use lsp_types::notification::*;
use lsp_types::*;

use lume_errors::Result;

use crate::backend::Backend;

pub const LSP_SOURCE_LUME: &str = "lume";

impl Backend {
    /// Drain all diagnostics from the inner diagnostics context to
    /// the language client.
    pub(crate) fn drain_dcx_diagnostics(&self, conn: &Connection) {
        self.dcx.with_iter(|diagnostics| {
            for diagnostic in diagnostics {
                self.publish_diagnostic(conn, diagnostic.as_ref());
            }
        });

        // Clear all the diagnostics from the context, so they won't
        // be reported on the next drain either.
        self.dcx.clear();

        // Take all the files which had one-or-more diagnostics, but no longer do and push
        // an empty list of diagnostics to the client.
        let prev = self.error_files_prev.read().unwrap();
        let curr = self.error_files_curr.read().unwrap();

        for file_url in prev.difference(&curr) {
            Self::publish_diagnostics_to_file(conn, &[], file_url.clone());
        }
    }

    /// Publishes the given [`error_snippet::Diagnostic`] to the language client.
    pub(crate) fn publish_diagnostic(&self, conn: &Connection, diagnostic: &dyn error_snippet::Diagnostic) {
        let Some(labels) = diagnostic.labels() else {
            return;
        };

        let labels = labels
            .into_iter()
            .filter_map(|label| self.lower_diagnostic_label(&label))
            .collect::<Vec<_>>();

        for label in &labels {
            self.error_files_curr
                .write()
                .unwrap()
                .insert(label.location.uri.clone());
        }

        let Some((primary_label, related)) = labels.split_first() else {
            return;
        };

        let related_info = related
            .iter()
            .map(|related| DiagnosticRelatedInformation {
                location: related.location.clone(),
                message: related.message.clone(),
            })
            .collect();

        let severity = match diagnostic.severity() {
            error_snippet::Severity::Note | error_snippet::Severity::Info => DiagnosticSeverity::INFORMATION,
            error_snippet::Severity::Help => DiagnosticSeverity::HINT,
            error_snippet::Severity::Warning => DiagnosticSeverity::WARNING,
            error_snippet::Severity::Error => DiagnosticSeverity::ERROR,
        };

        let code = diagnostic.code().map(|code| NumberOrString::String(code.to_string()));

        let mut message = primary_label.message.clone();
        if let Some(help_notes) = diagnostic.help() {
            for help_note in help_notes {
                let _ = write!(message, "\n{}", help_note.message);
            }
        }

        let diag = Diagnostic {
            range: primary_label.location.range,
            severity: Some(severity),
            code,
            code_description: None,
            source: Some(String::from(LSP_SOURCE_LUME)),
            message,
            related_information: Some(related_info),
            tags: None,
            data: None,
        };

        Self::publish_diagnostics_to_file(conn, &[diag], primary_label.location.uri.clone());
    }

    /// Publishes the given [`DiagnosticDiagnostic`] to the given file.
    pub(crate) fn publish_diagnostics_to_file(conn: &Connection, diag: &[Diagnostic], file: Uri) {
        let params = PublishDiagnosticsParams {
            uri: file,
            diagnostics: diag.to_vec(),
            version: None,
        };

        conn.sender
            .send(Message::Notification(lsp_server::Notification::new(
                PublishDiagnostics::METHOD.to_owned(),
                params,
            )))
            .unwrap();
    }

    /// Lower the given [`error_snippet::Label`] into a [`DiagnosticLabel`].
    ///
    /// If the label doesn't have any source content attached, [`None`] is returned.
    fn lower_diagnostic_label(&self, label: &error_snippet::Label) -> Option<DiagnosticLabel> {
        let source = label.source()?;
        let position = position_from_range(source.content().as_ref(), &label.range().0);

        let file_path = PathBuf::from(source.name()?);

        // Canonicalize the path to an absolute path, if not already.
        let url = if file_path.has_root() {
            Uri::from_str(file_path.to_str().unwrap()).unwrap()
        } else {
            let root = PathBuf::from(self.workspace_root.as_ref()?.as_str());
            let absolute = root.join(file_path.as_os_str().to_str().unwrap());

            Uri::from_str(absolute.to_str().unwrap()).unwrap()
        };

        Some(DiagnosticLabel {
            location: Location {
                uri: url,
                range: position,
            },
            message: label.message().to_owned(),
        })
    }
}

#[derive(Debug)]
struct DiagnosticLabel {
    pub location: Location,
    pub message: String,
}

fn position_from_range(text: &str, range: &std::ops::Range<usize>) -> Range {
    let start = position_from_index(text, range.start);
    let end = position_from_index(text, range.end);

    Range::new(start, end)
}

#[allow(clippy::cast_possible_truncation)]
fn position_from_index(text: &str, index: usize) -> Position {
    let mut line = 0;
    let mut line_start = 0;

    for (i, b) in text.bytes().enumerate() {
        if i == index {
            return Position::new(line, (i - line_start) as u32);
        }

        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }

    Position::new(line, (index - line_start) as u32)
}

pub(crate) trait IntoError {
    fn to_error(self) -> error_snippet::Error;
}

impl<T: std::error::Error + Send + Sync> IntoError for T {
    fn to_error(self) -> error_snippet::Error {
        Box::new(self).into_diagnostic()
    }
}

pub(crate) trait MapError<TResult> {
    fn map_error(self) -> Result<TResult>;
}

impl<T, E: std::error::Error + Send + Sync> MapError<T> for std::result::Result<T, E> {
    fn map_error(self) -> Result<T> {
        self.map_err(IntoError::to_error)
    }
}
