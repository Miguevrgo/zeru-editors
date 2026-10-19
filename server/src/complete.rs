//! Completion and signature help, read from the uses the parser recorded
//! where the cursor is: after a `.`, after a `::`, or anywhere else.

use std::collections::HashSet;

use lsp_types::{
    CompletionItem, CompletionItemKind, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation,
};

use crate::lexer::{KEYWORDS, Kind, PRIMITIVES, PRINTS, lex};
use crate::parser::{Item, ItemKind, LocalKind, UseKind};
use crate::program::{Def, Program, Ty, builtin_methods};

pub fn completion(program: &Program, file: usize, offset: usize) -> Vec<CompletionItem> {
    let text = &program.files[file].text;
    let start = text[..offset]
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_')
        .len();
    let head = &text[..start];
    let uses = &program.files[file].module.uses;
    if head.ends_with("::") {
        let qualifier = uses.iter().find_map(|used| match used.kind {
            UseKind::Path { qualifier, sep } if sep == start => Some(qualifier),
            _ => None,
        });
        return qualifier
            .and_then(|at| program.resolve(file, at))
            .map(|def| path_items(program, def))
            .unwrap_or_default();
    }
    if head.ends_with('.') && !head.ends_with("..") {
        let object = uses.iter().find_map(|used| match &used.kind {
            UseKind::Member { object, dot } if *dot == start => Some(object),
            _ => None,
        });
        return object
            .and_then(|object| program.type_of(file, object))
            .map(|ty| members(program, ty))
            .unwrap_or_default();
    }
    names(program, file, offset)
}

fn item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: (!detail.is_empty()).then(|| detail.to_string()),
        ..Default::default()
    }
}

fn kind_of(kind: ItemKind) -> CompletionItemKind {
    match kind {
        ItemKind::Fn => CompletionItemKind::FUNCTION,
        ItemKind::Method => CompletionItemKind::METHOD,
        ItemKind::Struct => CompletionItemKind::STRUCT,
        ItemKind::Enum => CompletionItemKind::ENUM,
        ItemKind::Trait => CompletionItemKind::INTERFACE,
        ItemKind::Const => CompletionItemKind::CONSTANT,
        ItemKind::Field => CompletionItemKind::FIELD,
        ItemKind::Variant => CompletionItemKind::ENUM_MEMBER,
    }
}

/// The items of `file` that `keep` accepts, `owner` being their struct or
/// enum, or none for the file's own items.
fn items_of(
    program: &Program,
    file: usize,
    owner: Option<usize>,
    keep: impl Fn(&Item) -> bool,
) -> Vec<CompletionItem> {
    let items = &program.files[file].module.items;
    items
        .iter()
        .filter(|entry| entry.owner == owner && keep(entry))
        .map(|entry| item(&entry.name, kind_of(entry.kind), &entry.decl))
        .collect()
}

fn named(names: &[&str], kind: CompletionItemKind) -> Vec<CompletionItem> {
    names.iter().map(|name| item(name, kind, "")).collect()
}

/// The fields and methods of a value, or the functions of a type.
fn members(program: &Program, ty: Ty) -> Vec<CompletionItem> {
    let method = CompletionItemKind::METHOD;
    match ty {
        Ty::Vec => named(&["new", "with_capacity"], method),
        Ty::Static(f, owner) => items_of(program, f, Some(owner), |m| {
            m.kind == ItemKind::Method && !m.has_self
        }),
        Ty::Value(f, ty) => match builtin_methods(&ty) {
            Some((methods, _)) => methods
                .iter()
                .map(|(name, _)| item(name, method, ""))
                .collect(),
            None => program
                .struct_of(f, &ty)
                .map(|(f, owner)| {
                    items_of(program, f, Some(owner), |m| {
                        m.kind == ItemKind::Field || m.has_self
                    })
                })
                .unwrap_or_default(),
        },
    }
}

/// The variants of an enum, the functions of a struct, or the public items
/// of a module.
fn path_items(program: &Program, def: Def) -> Vec<CompletionItem> {
    match def {
        Def::Module(f) => items_of(program, f, None, |entry| entry.is_pub),
        Def::Item(f, owner) => items_of(program, f, Some(owner), |entry| !entry.has_self),
        Def::Local(..) => Vec::new(),
    }
}

/// The locals in scope at `offset`, the items `file` sees, the keywords,
/// the primitive types and the print functions.
fn names(program: &Program, file: usize, offset: usize) -> Vec<CompletionItem> {
    let module = &program.files[file].module;
    let mut seen = HashSet::new();
    let mut found: Vec<CompletionItem> = program
        .locals_at(file, offset)
        .into_iter()
        .filter(|&at| seen.insert(&module.locals[at].name))
        .map(|at| {
            let kind = match module.locals[at].kind {
                LocalKind::TypeParam => CompletionItemKind::TYPE_PARAMETER,
                _ => CompletionItemKind::VARIABLE,
            };
            let detail = program.describe(Def::Local(file, at));
            item(&module.locals[at].name, kind, &detail)
        })
        .collect();
    found.extend(items_of(program, file, None, |_| true));
    for (import, target) in module.imports.iter().zip(&program.files[file].imports) {
        let name = import.path.last().map_or("", String::as_str);
        found.push(item(
            name,
            CompletionItemKind::MODULE,
            &import.path.join("."),
        ));
        if let Some(target) = *target {
            found.extend(items_of(program, target, None, |entry| {
                import.symbols.contains(&entry.name)
            }));
        }
    }
    if let Some(prelude) = program.prelude.filter(|&prelude| prelude != file) {
        found.extend(items_of(program, prelude, None, |entry| entry.is_pub));
    }
    found.extend(named(&KEYWORDS, CompletionItemKind::KEYWORD));
    found.extend(named(&PRIMITIVES, CompletionItemKind::KEYWORD));
    found.extend(named(&PRINTS, CompletionItemKind::FUNCTION));
    found
}

pub fn signature_help(program: &Program, file: usize, offset: usize) -> Option<SignatureHelp> {
    let text = &program.files[file].text;
    let tokens: Vec<_> = lex(&text[..offset])
        .into_iter()
        .filter(|t| t.kind != Kind::Comment)
        .collect();
    let mut depth = 0;
    let mut commas = 0;
    let mut open = None;
    for (at, token) in tokens.iter().enumerate().rev() {
        match &text[token.span.start..token.span.end] {
            _ if token.kind == Kind::Str => {}
            ")" | "]" | "}" => depth += 1,
            "(" if depth == 0 => {
                open = Some(at);
                break;
            }
            "(" | "[" | "{" if depth > 0 => depth -= 1,
            "[" | "{" | ";" if depth == 0 => return None,
            "," if depth == 0 => commas += 1,
            _ => {}
        }
    }
    let callee = tokens.get(open?.checked_sub(1)?)?.span;
    let uses = &program.files[file].module.uses;
    let used = uses.iter().position(|used| used.span == callee)?;
    let Def::Item(f, at) = program.resolve(file, used)? else {
        return None;
    };
    let function = &program.files[f].module.items[at];
    if !matches!(function.kind, ItemKind::Fn | ItemKind::Method) {
        return None;
    }
    let parameters = function.params.iter().map(|param| ParameterInformation {
        label: ParameterLabel::Simple(param.clone()),
        documentation: None,
    });
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: function.decl.clone(),
            documentation: None,
            parameters: Some(parameters.collect()),
            active_parameter: None,
        }],
        active_signature: Some(0),
        active_parameter: Some(commas),
    })
}
