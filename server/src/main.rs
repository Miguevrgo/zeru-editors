//! zeru-lsp, the Zeru language server: LSP over stdin and stdout. Requests
//! are answered on the main thread from a fresh parse of the open files and
//! what they import. Syntax errors are published as the user types; the
//! compiler's own errors come from `zeru check`, run on open and on save.

mod check;
mod complete;
mod lexer;
mod parser;
mod program;
mod tokens;

use std::collections::{HashMap, HashSet};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, DidSaveTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, Formatting, GotoDefinition, HoverRequest, InlayHintRequest,
    References, Rename, Request as _, SemanticTokensFullRequest, SignatureHelpRequest,
};
use lsp_types::{
    CompletionList, CompletionOptions, CompletionResponse, Diagnostic,
    DiagnosticRelatedInformation, DiagnosticSeverity, DocumentSymbol, DocumentSymbolResponse,
    GotoDefinitionResponse, Hover, HoverContents, HoverProviderCapability, InlayHint,
    InlayHintKind, InlayHintLabel, Location, MarkupContent, MarkupKind, OneOf, Position,
    PublishDiagnosticsParams, Range, SemanticTokens, SemanticTokensFullOptions,
    SemanticTokensLegend, SemanticTokensOptions, SemanticTokensResult, ServerCapabilities,
    SignatureHelpOptions, SymbolKind, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Uri,
    WorkspaceEdit,
};
use serde_json::{Value, json};

use lexer::Span;
use parser::{Hint, ItemKind, LocalKind};
use program::{BUILTINS, Program, Ty};

/// What the main loop answers: a message from the editor, which closing
/// its end turns into `exit`, or a finished check.
pub enum Event {
    Lsp(Message),
    Checked(check::Report),
}

/// The server's state: the open files, the diagnostics each source found,
/// and the checks under way.
struct Server {
    connection: Connection,
    events: Sender<Event>,
    docs: HashMap<PathBuf, String>,
    zeru: Option<PathBuf>,
    std: Option<PathBuf>,
    live: HashMap<PathBuf, Vec<Diagnostic>>,
    checked: HashMap<PathBuf, Vec<(PathBuf, Diagnostic)>>,
    taken: HashSet<PathBuf>,
    running: HashMap<PathBuf, check::Running>,
    generation: u64,
    snippets: bool,
}

fn main() -> io::Result<()> {
    let (connection, _threads) = Connection::stdio();
    let (id, params) = connection.initialize_start().map_err(io::Error::other)?;
    let zeru = check::find_zeru(params["initializationOptions"]["zeruPath"].as_str());
    let std = check::find_std(zeru.as_deref());
    let completion = &params["capabilities"]["textDocument"]["completion"];
    let snippets = completion["completionItem"]["snippetSupport"] == true;
    let initialized =
        json!({ "capabilities": capabilities(), "serverInfo": { "name": "zeru-lsp" } });
    connection
        .initialize_finish(id, initialized)
        .map_err(io::Error::other)?;

    let (events, received) = mpsc::channel();
    let forward = events.clone();
    let incoming = connection.receiver.clone();
    thread::spawn(move || {
        for message in incoming {
            if forward.send(Event::Lsp(message)).is_err() {
                return;
            }
        }
        let exit = Notification::new("exit".to_string(), ());
        let _ = forward.send(Event::Lsp(Message::Notification(exit)));
    });
    let mut server = Server {
        connection,
        events,
        docs: HashMap::new(),
        zeru,
        std,
        live: HashMap::new(),
        checked: HashMap::new(),
        taken: HashSet::new(),
        running: HashMap::new(),
        generation: 0,
        snippets,
    };
    for event in received {
        match event {
            Event::Lsp(Message::Request(request)) if request.method == "shutdown" => {
                server.send(Message::Response(Response::new_ok(request.id, ())));
            }
            Event::Lsp(Message::Request(request)) => {
                let id = request.id.clone();
                let response = catch_unwind(AssertUnwindSafe(|| server.respond(request)))
                    .unwrap_or_else(|_| {
                        let code = ErrorCode::InternalError as i32;
                        Response::new_err(id, code, "The analysis failed".to_string())
                    });
                server.send(Message::Response(response));
            }
            Event::Lsp(Message::Notification(notification)) if notification.method == "exit" => {
                break;
            }
            Event::Lsp(Message::Notification(notification)) => server.notified(notification),
            Event::Lsp(Message::Response(_)) => {}
            Event::Checked(report) => server.checked(report),
        }
    }
    server.running.values().for_each(check::Running::kill);
    Ok(())
}

fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                ..Default::default()
            },
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".into(), ":".into()]),
            ..Default::default()
        }),
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".into(), ",".into()]),
            ..Default::default()
        }),
        semantic_tokens_provider: Some(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: tokens::LEGEND.to_vec(),
                    token_modifiers: Vec::new(),
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                ..Default::default()
            }
            .into(),
        ),
        ..Default::default()
    }
}

fn answer<R: lsp_types::request::Request>(
    request: Request,
    handle: impl FnOnce(R::Params) -> R::Result,
) -> Response {
    match serde_json::from_value(request.params) {
        Ok(params) => Response::new_ok(request.id, handle(params)),
        Err(error) => Response::new_err(
            request.id,
            ErrorCode::InvalidParams as i32,
            error.to_string(),
        ),
    }
}

impl Server {
    fn send(&self, message: Message) {
        let _ = self.connection.sender.send(message);
    }

    fn respond(&self, request: Request) -> Response {
        match request.method.as_str() {
            HoverRequest::METHOD => {
                answer::<HoverRequest>(request, |p| self.hover(p.text_document_position_params))
            }
            GotoDefinition::METHOD => answer::<GotoDefinition>(request, |p| {
                self.definition(p.text_document_position_params)
            }),
            References::METHOD => answer::<References>(request, |p| {
                self.references(p.text_document_position, p.context.include_declaration)
            }),
            Rename::METHOD => answer::<Rename>(request, |p| {
                self.rename(p.text_document_position, p.new_name)
            }),
            DocumentSymbolRequest::METHOD => {
                answer::<DocumentSymbolRequest>(request, |p| self.symbols(&p.text_document.uri))
            }
            Formatting::METHOD => answer::<Formatting>(request, |p| {
                let (program, file) = self.program(&p.text_document.uri)?;
                let text = &program.files[file].text;
                let formatted = check::format(self.zeru.as_deref()?, text)?;
                let whole = Lines::new(text).range(Span::new(0, text.len()));
                Some(vec![TextEdit::new(whole, formatted)])
            }),
            InlayHintRequest::METHOD => answer::<InlayHintRequest>(request, |p| {
                let (program, file) = self.program(&p.text_document.uri)?;
                Some(type_hints(&program, file))
            }),
            Completion::METHOD => answer::<Completion>(request, |p| {
                let (program, file, offset) = self.locate(&p.text_document_position)?;
                let items = complete::completion(&program, file, offset, self.snippets);
                Some(CompletionResponse::List(CompletionList {
                    is_incomplete: items.is_empty(),
                    items,
                }))
            }),
            SignatureHelpRequest::METHOD => answer::<SignatureHelpRequest>(request, |p| {
                let (program, file, offset) = self.locate(&p.text_document_position_params)?;
                complete::signature_help(&program, file, offset)
            }),
            SemanticTokensFullRequest::METHOD => {
                answer::<SemanticTokensFullRequest>(request, |p| {
                    let (program, file) = self.program(&p.text_document.uri)?;
                    let data = tokens::semantic_tokens(&program, file);
                    Some(SemanticTokensResult::Tokens(SemanticTokens {
                        result_id: None,
                        data,
                    }))
                })
            }
            method => Response::new_err(
                request.id,
                ErrorCode::MethodNotFound as i32,
                format!("{method} is not supported"),
            ),
        }
    }

    /// The program made of every open file and what they import, and the
    /// index in it of the open file `uri` names.
    fn program(&self, uri: &Uri) -> Option<(Program, usize)> {
        let path = path_of(uri)?;
        let mut roots: Vec<PathBuf> = self.docs.keys().cloned().collect();
        roots.sort();
        let program = Program::load(&roots, self.std.as_deref(), |file| self.read(file));
        let file = program.find(&path)?;
        Some((program, file))
    }

    fn locate(&self, at: &TextDocumentPositionParams) -> Option<(Program, usize, usize)> {
        let (program, file) = self.program(&at.text_document.uri)?;
        let offset = Lines::new(&program.files[file].text).offset(at.position);
        Some((program, file, offset))
    }

    /// A file's text: the editor's, if it has the file open.
    fn read(&self, path: &Path) -> Option<String> {
        self.docs
            .get(path)
            .cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    fn hover(&self, at: TextDocumentPositionParams) -> Option<Hover> {
        let (program, file, offset) = self.locate(&at)?;
        let (span, defs) = program.meanings(file, offset)?;
        let range = Lines::new(&program.files[file].text).range(span);
        let shown: Vec<String> = defs.iter().map(|&def| program.markdown(def)).collect();
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: shown.join("\n\n---\n\n"),
            }),
            range: Some(range),
        })
    }

    fn definition(&self, at: TextDocumentPositionParams) -> Option<GotoDefinitionResponse> {
        let (program, file, offset) = self.locate(&at)?;
        let (_, defs) = program.meanings(file, offset)?;
        let mut found: Vec<Location> = defs
            .into_iter()
            .map(|def| program.declared(def))
            .filter(|&(file, _)| file != BUILTINS)
            .map(|(file, span)| location(&program, file, span))
            .collect();
        Some(match found.len() {
            1 => GotoDefinitionResponse::Scalar(found.remove(0)),
            _ => GotoDefinitionResponse::Array(found),
        })
    }

    fn references(
        &self,
        at: TextDocumentPositionParams,
        with_declaration: bool,
    ) -> Option<Vec<Location>> {
        let (program, file, offset) = self.locate(&at)?;
        let (_, def) = program.at(file, offset)?;
        let declared = program.declared(def);
        let found = program.occurrences(def).into_iter();
        let found = found.filter(|&at| (with_declaration || at != declared) && at.0 != BUILTINS);
        Some(
            found
                .map(|(file, span)| location(&program, file, span))
                .collect(),
        )
    }

    /// Rename what the name at the cursor refers to, everywhere the program
    /// names it, unless the standard library declares it.
    #[allow(clippy::mutable_key_type)]
    fn rename(&self, at: TextDocumentPositionParams, name: String) -> Option<WorkspaceEdit> {
        let (program, file, offset) = self.locate(&at)?;
        let (_, def) = program.at(file, offset)?;
        if program.files[program.declared(def).0].std || matches!(def, program::Def::Module(_)) {
            return None;
        }
        let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
        for (file, span) in program.occurrences(def) {
            let found = location(&program, file, span);
            changes
                .entry(found.uri)
                .or_default()
                .push(TextEdit::new(found.range, name.clone()));
        }
        Some(WorkspaceEdit::new(changes))
    }

    fn symbols(&self, uri: &Uri) -> Option<DocumentSymbolResponse> {
        let (program, file) = self.program(uri)?;
        let lines = Lines::new(&program.files[file].text);
        let items = &program.files[file].module.items;
        #[allow(deprecated)]
        let symbol = |at: usize, children: Option<Vec<DocumentSymbol>>| DocumentSymbol {
            name: items[at].name.clone(),
            detail: Some(items[at].decl.clone()),
            kind: match items[at].kind {
                ItemKind::Fn => SymbolKind::FUNCTION,
                ItemKind::Method => SymbolKind::METHOD,
                ItemKind::Struct => SymbolKind::STRUCT,
                ItemKind::Enum => SymbolKind::ENUM,
                ItemKind::Trait => SymbolKind::INTERFACE,
                ItemKind::Const => SymbolKind::CONSTANT,
                ItemKind::Field => SymbolKind::FIELD,
                ItemKind::Variant => SymbolKind::ENUM_MEMBER,
            },
            tags: None,
            deprecated: None,
            range: lines.range(items[at].span),
            selection_range: lines.range(items[at].name_span),
            children,
        };
        let top = (0..items.len()).filter(|&at| items[at].owner.is_none());
        let found = top.map(|at| {
            let children = program.children(file, at).map(|child| symbol(child, None));
            symbol(at, Some(children.collect()))
        });
        Some(DocumentSymbolResponse::Nested(found.collect()))
    }

    fn notified(&mut self, notification: Notification) {
        let params = notification.params;
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let Ok(params) =
                    serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(params)
                else {
                    return;
                };
                let Some(path) = path_of(&params.text_document.uri) else {
                    return;
                };
                self.docs.insert(path.clone(), params.text_document.text);
                self.lint(&path);
                self.check(&path);
            }
            DidChangeTextDocument::METHOD => {
                let Ok(mut params) =
                    serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(params)
                else {
                    return;
                };
                let (Some(path), Some(change)) = (
                    path_of(&params.text_document.uri),
                    params.content_changes.pop(),
                ) else {
                    return;
                };
                self.docs.insert(path.clone(), change.text);
                self.taken.remove(&path);
                self.lint(&path);
            }
            DidSaveTextDocument::METHOD => {
                let Ok(params) =
                    serde_json::from_value::<lsp_types::DidSaveTextDocumentParams>(params)
                else {
                    return;
                };
                if let Some(path) = path_of(&params.text_document.uri) {
                    self.check(&path);
                }
            }
            DidCloseTextDocument::METHOD => {
                let Ok(params) =
                    serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(params)
                else {
                    return;
                };
                if let Some(path) = path_of(&params.text_document.uri) {
                    self.docs.remove(&path);
                    self.live.remove(&path);
                    self.taken.remove(&path);
                    self.publish(&path);
                }
            }
            _ => {}
        }
    }

    /// Publish the syntax errors of the open file at `path`.
    fn lint(&mut self, path: &Path) {
        let text = &self.docs[path];
        let lines = Lines::new(text);
        let errors = parser::parse(text).errors;
        let found = errors
            .into_iter()
            .map(|(span, message)| error(lines.range(span), message));
        self.live.insert(path.to_path_buf(), found.collect());
        self.publish(path);
    }

    /// Start checking `path` with the compiler, stopping a check of it that
    /// is still under way.
    fn check(&mut self, path: &Path) {
        let Some(zeru) = &self.zeru else {
            return;
        };
        if let Some(old) = self.running.remove(path) {
            old.kill();
        }
        self.generation += 1;
        if let Ok(running) = check::spawn(zeru, path, self.generation, self.events.clone()) {
            self.running.insert(path.to_path_buf(), running);
        }
    }

    /// Take in the errors of the latest check of a file, which then replace
    /// its live syntax errors until it changes again, if the check read what
    /// the editor holds.
    fn checked(&mut self, report: check::Report) {
        let current = self
            .running
            .get(&report.root)
            .map(|running| running.generation);
        if current != Some(report.generation) {
            return;
        }
        self.running.remove(&report.root);
        let dir = report.root.parent().unwrap_or(Path::new("/"));
        let found: Vec<(PathBuf, Diagnostic)> = report
            .errors
            .iter()
            .filter(|found| found["message"] != "Missing 'fn main'")
            .filter_map(|found| {
                let (path, range) = Self::place(dir, found)?;
                let notes = found["notes"].as_array().into_iter().flatten();
                let related = notes.filter_map(|note| {
                    let (path, range) = Self::place(dir, note)?;
                    Some(DiagnosticRelatedInformation {
                        location: Location::new(uri_of(&path), range),
                        message: note["message"].as_str()?.to_string(),
                    })
                });
                let mut diagnostic = error(range, found["message"].as_str()?.to_string());
                diagnostic.related_information = Some(related.collect());
                Some((path, diagnostic))
            })
            .collect();
        let old = self
            .checked
            .insert(report.root.clone(), found)
            .unwrap_or_default();
        if std::fs::read_to_string(&report.root).ok().as_ref() == self.docs.get(&report.root) {
            self.taken.insert(report.root.clone());
        }
        let mut touched: Vec<PathBuf> = old.into_iter().map(|(path, _)| path).collect();
        touched.extend(
            self.checked[&report.root]
                .iter()
                .map(|(path, _)| path.clone()),
        );
        touched.push(report.root);
        touched.sort();
        touched.dedup();
        for path in touched {
            self.publish(&path);
        }
    }

    /// Where an error the compiler printed is: its file, relative to `dir`
    /// where it ran, and its range in the text it read, from 1-based lines
    /// and byte columns.
    fn place(dir: &Path, found: &Value) -> Option<(PathBuf, Range)> {
        let path = dir.join(found["file"].as_str()?);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let lines = Lines::new(&text);
        let at = |line: &str, column: &str| {
            Some(lines.at(found[line].as_u64()?, found[column].as_u64()?))
        };
        let span = Span::new(at("line", "column")?, at("end_line", "end_column")?);
        Some((path, lines.range(span)))
    }

    /// Publish what the compiler last found in `path`, and its live syntax
    /// errors unless a check has replaced them since it last changed.
    fn publish(&self, path: &Path) {
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let checked = self
            .checked
            .values()
            .flatten()
            .filter(|(file, _)| file == path);
        let live = self
            .live
            .get(path)
            .filter(|_| !self.taken.contains(path))
            .into_iter()
            .flatten();
        for diagnostic in checked.map(|(_, diagnostic)| diagnostic).chain(live) {
            if !diagnostics.contains(diagnostic) {
                diagnostics.push(diagnostic.clone());
            }
        }
        let params = PublishDiagnosticsParams {
            uri: uri_of(path),
            diagnostics,
            version: None,
        };
        let notification = Notification::new(PublishDiagnostics::METHOD.to_string(), params);
        self.send(Message::Notification(notification));
    }
}

fn error(range: Range, message: String) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("zeru".to_string()),
        message,
        ..Default::default()
    }
}

fn location(program: &Program, file: usize, span: Span) -> Location {
    let range = Lines::new(&program.files[file].text).range(span);
    Location::new(uri_of(&program.files[file].path), range)
}

fn path_of(uri: &Uri) -> Option<PathBuf> {
    let path = uri.path().as_estr().decode().into_string().ok()?;
    Some(PathBuf::from(path.into_owned()))
}

fn uri_of(path: &Path) -> Uri {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri.parse().expect("an encoded path is a URI")
}

/// The types the source leaves out, shown after each name as `: T`, as
/// rust-analyzer's inlay hints.
fn type_hints(program: &Program, file: usize) -> Vec<InlayHint> {
    let lines = Lines::new(&program.files[file].text);
    let locals = program.files[file].module.locals.iter().enumerate();
    locals
        .filter(|(_, local)| matches!(local.kind, LocalKind::Var | LocalKind::Binding))
        .filter(|(_, local)| !matches!(local.hint, Some(Hint::Written(_))))
        .filter_map(|(at, local)| {
            let Some(Ty::Value(_, ty)) = program.local_type(file, at, 0) else {
                return None;
            };
            Some(InlayHint {
                position: lines.position(local.span.end),
                label: InlayHintLabel::String(format!(": {ty}")),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: None,
                padding_left: None,
                padding_right: None,
                data: None,
            })
        })
        .collect()
}

/// A file's text and where its lines start, to turn byte offsets into LSP
/// positions, whose columns count UTF-16 units, and back.
pub struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
}

impl<'t> Lines<'t> {
    pub fn new(text: &'t str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Self { text, starts }
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let column = self.text[self.starts[line]..offset].encode_utf16().count();
        Position::new(line as u32, column as u32)
    }

    pub fn offset(&self, position: Position) -> usize {
        let Some(&start) = self.starts.get(position.line as usize) else {
            return self.text.len();
        };
        let mut units = 0;
        for (at, ch) in self.text[start..].char_indices() {
            if units >= position.character as usize || ch == '\n' {
                return start + at;
            }
            units += ch.len_utf16();
        }
        self.text.len()
    }

    /// The offset of a 1-based line and byte column.
    pub fn at(&self, line: u64, column: u64) -> usize {
        let start = self.starts.get((line as usize).saturating_sub(1)).copied();
        let offset = start.unwrap_or(self.text.len()) + (column as usize).saturating_sub(1);
        self.text.floor_char_boundary(offset)
    }

    pub fn range(&self, span: Span) -> Range {
        Range::new(self.position(span.start), self.position(span.end))
    }
}
