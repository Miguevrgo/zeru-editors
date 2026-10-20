//! Completion and signature help, read from the uses the parser recorded
//! where the cursor is: after a `.`, after a `::`, or anywhere else.

use std::collections::HashSet;

use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, Documentation,
    InsertTextFormat, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel,
    SignatureHelp, SignatureInformation,
};

use crate::lexer::{KEYWORDS, Kind, PRIMITIVES, lex};
use crate::parser::{Item, ItemKind, LocalKind, UseKind};
use crate::program::{BUILTINS, Def, Program, Ty};

pub fn completion(
    program: &Program,
    file: usize,
    offset: usize,
    snippets: bool,
) -> Vec<CompletionItem> {
    let text = &program.files[file].text;
    let start = text[..offset]
        .trim_end_matches(|c: char| c.is_ascii_alphanumeric() || c == '_')
        .len();
    let head = &text[..start];
    if head.ends_with(':') && !head.ends_with("::") {
        return Vec::new();
    }
    let line = &head[head.rfind('\n').map_or(0, |at| at + 1)..];
    let uses = &program.files[file].module.uses;
    let mut found = if let Some(found) = import_names(program, file, line) {
        found
    } else if head.ends_with("::") {
        let qualifier = uses.iter().find_map(|used| match used.kind {
            UseKind::Path { qualifier, sep } if sep == start => Some(qualifier),
            _ => None,
        });
        qualifier
            .and_then(|at| program.resolve(file, at))
            .map(|def| path_items(program, def))
            .unwrap_or_default()
    } else if head.ends_with('.') && !head.ends_with("..") {
        let object = uses.iter().find_map(|used| match &used.kind {
            UseKind::Member { object, dot } if *dot == start => Some(object),
            _ => None,
        });
        object
            .and_then(|object| program.type_of(file, object))
            .map(|ty| members(program, ty))
            .unwrap_or_default()
    } else {
        names(program, file, offset)
    };
    for item in found.iter_mut().filter(|_| !snippets) {
        if item.insert_text_format.take().is_some() {
            item.insert_text = Some(format!("{}(", item.label));
        }
    }
    found
}

/// What an `import` line names next: the modules under its path after a
/// `.`, or the module's public items after `::`.
fn import_names(program: &Program, file: usize, line: &str) -> Option<Vec<CompletionItem>> {
    let path = line.trim_start().strip_prefix("import ")?;
    let (path, symbols) = match path.split_once("::") {
        Some((path, _)) => (path.trim(), true),
        None => (path.trim_end_matches(|c: char| c != '.'), false),
    };
    let parts: Vec<&str> = path.split('.').filter(|part| !part.is_empty()).collect();
    let std = program.prelude.and_then(|at| program.files[at].path.parent());
    let dir = match parts.split_first() {
        Some((&"std", rest)) => std?.join(rest.join("/")),
        _ => program.files[file].path.parent()?.join(parts.join("/")),
    };
    if symbols {
        let target = program.find(&dir.with_extension("zr"))?;
        return Some(items_of(program, target, None, 1, |item| item.is_pub));
    }
    let mut found: Vec<CompletionItem> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let module = path.is_dir() || path.extension().is_some_and(|ext| ext == "zr");
            let name = path.file_stem()?.to_str()?.to_string();
            (module && name != "builtin" && path != program.files[file].path).then_some(name)
        })
        .map(|name| entry(&name, CompletionItemKind::MODULE, 1))
        .collect();
    if parts.is_empty() {
        found.push(entry("std", CompletionItemKind::MODULE, 0));
    }
    Some(found)
}

/// An item labelled `label`, sorted in `group`: locals first, then members,
/// the file's items, imports, the prelude and the keywords; constants after
/// the rest of their group.
fn entry(label: &str, kind: CompletionItemKind, group: u8) -> CompletionItem {
    let rank = u8::from(kind == CompletionItemKind::CONSTANT);
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        sort_text: Some(format!("{group}{rank}{label}")),
        ..Default::default()
    }
}

/// The completion of what `def` names, as rust-analyzer gives it: its
/// parameters or type beside the label, its signature and docs, and for a
/// function a snippet with a placeholder per parameter.
fn declared(program: &Program, def: Def, group: u8) -> CompletionItem {
    let (label, kind, params, ty) = match def {
        Def::Item(f, at) => {
            let item = &program.files[f].module.items[at];
            let kind = match item.kind {
                ItemKind::Fn => CompletionItemKind::FUNCTION,
                ItemKind::Method => CompletionItemKind::METHOD,
                ItemKind::Struct => CompletionItemKind::STRUCT,
                ItemKind::Enum => CompletionItemKind::ENUM,
                ItemKind::Trait => CompletionItemKind::INTERFACE,
                ItemKind::Const => CompletionItemKind::CONSTANT,
                ItemKind::Field => CompletionItemKind::FIELD,
                ItemKind::Variant => CompletionItemKind::ENUM_MEMBER,
            };
            let callable = matches!(item.kind, ItemKind::Fn | ItemKind::Method);
            let params = (callable || !item.params.is_empty()).then_some(&item.params);
            (item.name.as_str(), kind, params, item.ty.clone())
        }
        Def::Local(f, at) => {
            let local = &program.files[f].module.locals[at];
            let kind = match local.kind {
                LocalKind::TypeParam => CompletionItemKind::TYPE_PARAMETER,
                _ => CompletionItemKind::VARIABLE,
            };
            let ty = match program.local_type(f, at, 0) {
                Some(Ty::Value(_, ty)) => Some(ty),
                _ => None,
            };
            (local.name.as_str(), kind, None, ty)
        }
        Def::Module(f) => {
            let name = program.files[f]
                .path
                .file_stem()
                .and_then(|stem| stem.to_str());
            (
                name.unwrap_or_default(),
                CompletionItemKind::MODULE,
                None,
                None,
            )
        }
    };
    let mut item = entry(label, kind, group);
    item.detail = Some(program.describe(def));
    item.documentation = Some(Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: program.markdown(def),
    }));
    item.label_details = Some(CompletionItemLabelDetails {
        detail: params.map(|params| format!("({})", params.join(", "))),
        description: ty,
    });
    if let Some(params) = params.filter(|_| kind != CompletionItemKind::ENUM_MEMBER) {
        let holes = params.iter().enumerate().map(|(at, param)| {
            let name = param.split(':').next().unwrap_or_default();
            format!("${{{}:{}}}", at + 1, name.trim_start_matches("var ").trim())
        });
        let holes: Vec<String> = holes.collect();
        let end = if holes.is_empty() { "$0" } else { "" };
        item.insert_text = Some(format!("{label}({}){end}", holes.join(", ")));
        item.insert_text_format = Some(InsertTextFormat::SNIPPET);
    }
    item
}

/// The items of `file` that `keep` accepts, `owner` being their struct or
/// enum, or none for the file's own items.
fn items_of(
    program: &Program,
    file: usize,
    owner: Option<usize>,
    group: u8,
    keep: impl Fn(&Item) -> bool,
) -> Vec<CompletionItem> {
    let items = program.files[file].module.items.iter().enumerate();
    items
        .filter(|(_, item)| item.owner == owner && keep(item))
        .map(|(at, _)| declared(program, Def::Item(file, at), group))
        .collect()
}

/// The fields and methods of a value, or the functions of a type.
fn members(program: &Program, ty: Ty) -> Vec<CompletionItem> {
    let (f, owner, on_value) = match ty {
        Ty::Static(f, owner) => (f, owner, false),
        Ty::Value(f, ty) => match program.struct_of(f, &ty) {
            Some((f, owner)) => (f, owner, true),
            None => return Vec::new(),
        },
    };
    items_of(program, f, Some(owner), 1, |member| match on_value {
        true => member.kind == ItemKind::Field || member.has_self,
        false => member.kind == ItemKind::Method && !member.has_self,
    })
}

/// The variants of an enum, the functions of a struct, or the public items
/// of a module.
fn path_items(program: &Program, def: Def) -> Vec<CompletionItem> {
    match def {
        Def::Module(f) => items_of(program, f, None, 1, |item| item.is_pub),
        Def::Item(f, owner) => items_of(program, f, Some(owner), 1, |item| !item.has_self),
        Def::Local(..) => Vec::new(),
    }
}

/// The locals in scope at `offset`, the items `file` sees, its imports, the
/// prelude's and the compiler's public items, the keywords and the
/// primitive types.
fn names(program: &Program, file: usize, offset: usize) -> Vec<CompletionItem> {
    let module = &program.files[file].module;
    let mut seen = HashSet::new();
    let locals = program.locals_at(file, offset).into_iter();
    let locals = locals.filter(|&at| seen.insert(&module.locals[at].name));
    let mut found: Vec<CompletionItem> = locals
        .map(|at| declared(program, Def::Local(file, at), 0))
        .collect();
    found.extend(items_of(program, file, None, 2, |_| true));
    for (import, target) in module.imports.iter().zip(&program.files[file].imports) {
        let Some(target) = *target else {
            continue;
        };
        found.push(declared(program, Def::Module(target), 3));
        found.extend(items_of(program, target, None, 3, |item| {
            import.symbols.contains(&item.name)
        }));
    }
    for shared in program.prelude.into_iter().chain([BUILTINS]) {
        if shared != file {
            found.extend(items_of(program, shared, None, 4, |item| item.is_pub));
        }
    }
    let words = KEYWORDS.iter().chain(&PRIMITIVES);
    found.extend(words.map(|word| entry(word, CompletionItemKind::KEYWORD, 5)));
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
    let docs = (!function.docs.is_empty()).then(|| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: function.docs.clone(),
        })
    });
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: function.decl.clone(),
            documentation: docs,
            parameters: Some(parameters.collect()),
            active_parameter: None,
        }],
        active_signature: Some(0),
        active_parameter: Some(commas),
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    const SOURCE: &str = "import std.collections.hashmap::{HashMap};
fn main() {
    var m: HashMap<i64, str> = HashMap.new();
    m.get(1);
    var v: Vec<i32> = Vec.new();
    v.push(1);
    println(\"{}\", v.len());
    exit(0);
}
";

    #[test]
    fn docs_signatures_and_snippets() {
        let std = crate::check::find_std(crate::check::find_zeru(None).as_deref());
        let root = PathBuf::from("/p/main.zr");
        let read = |path: &Path| match path == root {
            true => Some(SOURCE.to_string()),
            false => std::fs::read_to_string(path).ok(),
        };
        let program = Program::load(std::slice::from_ref(&root), std.as_deref(), read);
        let main = program.find(&root).unwrap();
        let at = |needle: &str| SOURCE.find(needle).unwrap() + needle.len();
        let hover = |needle| {
            let (_, defs) = program.meanings(main, at(needle)).unwrap();
            program.markdown(defs[0])
        };
        assert_eq!(
            hover("m.get"),
            "```zeru\nHashMap\npub fn get(self, key: K) (&V)?\n```\n\n---\n\nThe value under `key`, if any; `.copy()` gives a copy of it."
        );
        assert!(hover("v.push").contains("Appends `value` at the end"));
        assert!(hover("println").contains("Like print, then a newline"));
        assert!(hover("exit").contains("End the process"));

        let found = completion(&program, main, at("v."), true);
        let push = found.iter().find(|item| item.label == "push").unwrap();
        assert_eq!(push.kind, Some(CompletionItemKind::METHOD));
        assert_eq!(push.detail.as_deref(), Some("fn push(var self, value: T)"));
        let details = push.label_details.as_ref().unwrap();
        assert_eq!(details.detail.as_deref(), Some("(value: T)"));
        assert_eq!(push.insert_text.as_deref(), Some("push(${1:value})"));
        assert_eq!(push.insert_text_format, Some(InsertTextFormat::SNIPPET));
        let Some(Documentation::MarkupContent(docs)) = &push.documentation else {
            panic!("push has no documentation");
        };
        assert!(docs.value.contains("Vec\nfn push(var self, value: T)"));
        let len = found.iter().find(|item| item.label == "len").unwrap();
        assert_eq!(len.insert_text.as_deref(), Some("len()$0"));
        let description = len
            .label_details
            .as_ref()
            .and_then(|d| d.description.as_deref());
        assert_eq!(description, Some("usize"));

        let plain = completion(&program, main, at("v."), false);
        let push = plain.iter().find(|item| item.label == "push").unwrap();
        assert_eq!(push.insert_text.as_deref(), Some("push("));
        assert_eq!(push.insert_text_format, None);

        let names = completion(&program, main, at("println(\"{}\", ") - 1, true);
        let sorted = |label: &str| {
            let item = names.iter().find(|item| item.label == label).unwrap();
            item.sort_text.clone().unwrap()
        };
        assert!(sorted("v") < sorted("main") && sorted("main") < sorted("exit"));
        assert!(sorted("exit") < sorted("while"));
    }

    #[test]
    fn imports_and_functions_before_constants() {
        let std = crate::check::find_std(crate::check::find_zeru(None).as_deref());
        let root = PathBuf::from("/p/main.zr");
        let labels = |source: &str, offset: usize| {
            let source = source.to_string();
            let read = |path: &Path| match path == root {
                true => Some(source.clone()),
                false => std::fs::read_to_string(path).ok(),
            };
            let program = Program::load(std::slice::from_ref(&root), std.as_deref(), read);
            let main = program.find(&root).unwrap();
            let mut found = completion(&program, main, offset, true);
            found.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
            found.into_iter().map(|item| item.label).collect::<Vec<_>>()
        };
        let modules = labels("import std.\n", 11);
        assert!(modules.contains(&"fs".into()) && modules.contains(&"collections".into()));
        assert!(!modules.contains(&"builtin".into()));
        assert!(labels("import std.collections.\n", 23).contains(&"hashmap".into()));
        let source = "import std.fs::{};\nfn main() { fs::open(\"p\", 0, 0); }\n";
        assert!(labels(source, 16).contains(&"read_file".into()));
        let members = labels(source, source.find("fs::").unwrap() + 4);
        let open = members.iter().position(|label| label == "open").unwrap();
        assert!(open < members.iter().position(|label| label == "O_RDONLY").unwrap());
    }
}
