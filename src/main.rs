use std::error::Error;

use clap::{ArgAction, Parser, ValueHint};
use lsp_server::Connection;
use lsp_types::*;
use lume_errors::Result;

pub(crate) mod backend;

#[derive(Debug, Parser)]
#[clap(
    name = "lume-lsp",
    version = env!("CARGO_PKG_VERSION"),
    about = "Language Server for Lume and Arcfiles",
    long_about = None
)]
#[command(allow_missing_positional(true))]
pub(crate) struct LumeLspCli {
    #[arg(long, help = "Writes log output to the given file", value_hint = ValueHint::FilePath)]
    pub log_file: Option<String>,

    #[arg(long, help = "Writes log output to standard output")]
    pub log_stdout: bool,

    #[arg(long, short = 'v', help = "Enables verbose output", action = ArgAction::Count)]
    pub verbose: u8,
}

fn main() -> std::result::Result<(), Box<dyn Error + Sync + Send>> {
    let mut args = LumeLspCli::parse();

    let level_filter = match args.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };

    // If the user defined `-v` but no logging was defined, assume standard output.
    if args.verbose > 0 && !args.log_stdout && args.log_file.is_none() {
        args.log_stdout = true;
    }

    if let Some(log_file) = args.log_file {
        simple_logging::log_to_file(log_file, level_filter)?;
    }

    if args.log_stdout {
        simple_logging::log_to(std::io::stdout(), level_filter);
    }

    lume_lsp_entry()
}

fn lume_lsp_entry() -> std::result::Result<(), Box<dyn Error + Sync + Send>> {
    let (conn, io) = Connection::stdio();

    let capabilities = ServerCapabilities {
        completion_provider: Some(CompletionOptions {
            resolve_provider: Some(false),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        text_document_sync: Some(TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: Some(TextDocumentSyncKind::FULL),
            save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                include_text: Some(true),
            })),
            ..Default::default()
        })),
        ..Default::default()
    };

    log::info!("starting up!");

    let params_json = conn.initialize(serde_json::json!(capabilities))?;
    let params = serde_json::from_value(params_json)?;

    if let Err(err) = initialize(&conn, params) {
        return Err(Box::new(std::io::Error::other(err.message())));
    }

    io.join()?;
    log::info!("shutting down server...");

    Ok(())
}

fn initialize(connection: &Connection, params: InitializeParams) -> Result<()> {
    let mut backend = backend::Backend::initialize(params);

    backend.listen(connection)
}
