use lsp_server::RequestId;
use lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, Position};
use lume_errors::Result;

use crate::state::State;

pub(crate) fn on_hover(state: &State, id: RequestId, params: HoverParams) -> Result<()> {
    let uri = &params.text_document_position_params.text_document.uri;
    let Position { line, character } = params.text_document_position_params.position;

    let Some(location) = state.location_of(uri, line as usize, character as usize) else {
        state.err(id, lsp_server::ErrorCode::InvalidParams, "document not available")?;
        return Ok(());
    };

    let content = match crate::symbols::hover::hover_content_of(state, location) {
        Ok(content) => content,
        Err(err) => {
            log::error!("could not retrieve content: {}", err.message());

            state.err(
                id,
                lsp_server::ErrorCode::RequestFailed,
                &format!("could not retrieve content: {}", err.message()),
            )?;
            return Ok(());
        }
    };

    if content.is_empty() {
        log::warn!("no content for {location}");
    }

    state.ok(id, &Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })?;

    Ok(())
}
