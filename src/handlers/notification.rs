use std::sync::Arc;

use lsp_types::*;
use lume_span::SourceFile;

use crate::state::State;

pub(crate) fn open_document(state: &mut State, params: DidOpenTextDocumentParams) {
    log::info!("added document {}", params.text_document.uri.as_str());

    let TextDocumentItem { uri, text, .. } = params.text_document;
    let Some(source_file) = state.source_of_uri(&uri) else {
        log::error!("could not find any matching package");
        return;
    };

    state.vfs.add_document(
        uri,
        Arc::new(SourceFile {
            id: source_file.id,
            name: source_file.name.clone(),
            content: text,
            package: source_file.package,
        }),
    );

    state.compile_workspace();
}

pub(crate) fn close_document(state: &mut State, params: DidCloseTextDocumentParams) {
    log::info!("removed document {}", params.text_document.uri.as_str());

    state.vfs.remove_document(&params.text_document.uri);

    state.compile_workspace();
}

pub(crate) fn save_document(state: &mut State, params: DidSaveTextDocumentParams) {
    log::info!("updated document {} (via save)", params.text_document.uri.as_str());

    state
        .vfs
        .change_document(&params.text_document.uri, params.text.unwrap());

    state.compile_workspace();
}

pub(crate) fn change_document(state: &mut State, params: DidChangeTextDocumentParams) {
    log::info!("updated document {} (via change)", params.text_document.uri.as_str());

    let source = params.content_changes.first().unwrap().text.clone();
    state.vfs.change_document(&params.text_document.uri, source);

    state.compile_workspace();
}
