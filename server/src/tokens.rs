//! Semantic tokens: each name classed by what it refers to, every other
//! token by its kind, and each comment.

use std::collections::HashMap;

use lsp_types::{SemanticToken, SemanticTokenType as T};

use crate::Lines;
use crate::lexer::{Kind, PRIMITIVES, PRINTS};
use crate::parser::{ItemKind, LocalKind, UseKind};
use crate::program::{Def, Program};

pub const LEGEND: [T; 18] = [
    T::NAMESPACE,
    T::TYPE,
    T::STRUCT,
    T::ENUM,
    T::INTERFACE,
    T::TYPE_PARAMETER,
    T::ENUM_MEMBER,
    T::FUNCTION,
    T::METHOD,
    T::VARIABLE,
    T::PARAMETER,
    T::PROPERTY,
    T::KEYWORD,
    T::NUMBER,
    T::STRING,
    T::COMMENT,
    T::OPERATOR,
    T::MACRO,
];

const PUNCTUATION: [&str; 11] = ["(", ")", "{", "}", "[", "]", ",", ";", ":", "::", "."];

fn class(program: &Program, def: Def) -> T {
    match def {
        Def::Local(f, at) => match program.files[f].module.locals[at].kind {
            LocalKind::Param => T::PARAMETER,
            LocalKind::TypeParam => T::TYPE_PARAMETER,
            LocalKind::Var | LocalKind::Binding => T::VARIABLE,
        },
        Def::Item(f, at) => match program.files[f].module.items[at].kind {
            ItemKind::Fn => T::FUNCTION,
            ItemKind::Method => T::METHOD,
            ItemKind::Struct => T::STRUCT,
            ItemKind::Enum => T::ENUM,
            ItemKind::Trait => T::INTERFACE,
            ItemKind::Const => T::VARIABLE,
            ItemKind::Field => T::PROPERTY,
            ItemKind::Variant => T::ENUM_MEMBER,
        },
        Def::Module(_) => T::NAMESPACE,
    }
}

pub fn semantic_tokens(program: &Program, file: usize) -> Vec<SemanticToken> {
    let source = &program.files[file];
    let module = &source.module;
    let text = source.text.as_str();
    let called = |end: usize| text[end..].trim_start().starts_with('(');
    let mut names = HashMap::new();
    for (at, item) in module.items.iter().enumerate() {
        names.insert(item.name_span.start, class(program, Def::Item(file, at)));
    }
    for (at, local) in module.locals.iter().enumerate() {
        names.insert(local.span.start, class(program, Def::Local(file, at)));
    }
    for (at, used) in module.uses.iter().enumerate() {
        let found = match (program.resolve(file, at), &used.kind) {
            (Some(def), _) => class(program, def),
            (None, UseKind::Member { .. }) if called(used.span.end) => T::METHOD,
            (None, UseKind::Member { .. } | UseKind::Field { .. }) => T::PROPERTY,
            (None, UseKind::Name | UseKind::Type) => match &text[used.span.start..used.span.end] {
                name if PRIMITIVES.contains(&name) || name == "Vec" || name == "Array" => T::TYPE,
                name if PRINTS.contains(&name) => T::MACRO,
                "Some" | "Ok" | "Err" => T::ENUM_MEMBER,
                _ if text[used.span.end..].starts_with("::") => T::NAMESPACE,
                _ if called(used.span.end) => T::FUNCTION,
                _ => continue,
            },
            _ => continue,
        };
        names.insert(used.span.start, found);
    }

    let lines = Lines::new(text);
    let mut data = Vec::new();
    let (mut line, mut column) = (0, 0);
    for token in &module.tokens {
        let word = &text[token.span.start..token.span.end];
        let kind = match &token.kind {
            _ if let Some(kind) = names.get(&token.span.start) => kind.clone(),
            Kind::Comment => T::COMMENT,
            Kind::Str => T::STRING,
            Kind::Int | Kind::Float => T::NUMBER,
            Kind::Keyword if word == "str" => T::TYPE,
            Kind::Keyword if word == "None" => T::ENUM_MEMBER,
            Kind::Keyword => T::KEYWORD,
            Kind::Op if !PUNCTUATION.contains(&word) => T::OPERATOR,
            _ => continue,
        };
        let token_type = LEGEND
            .iter()
            .position(|legend| *legend == kind)
            .unwrap_or_default() as u32;
        let mut from = token.span.start;
        for piece in word.split('\n') {
            let at = lines.position(from);
            from += piece.len() + 1;
            if piece.is_empty() {
                continue;
            }
            let delta_line = at.line - line;
            let delta_start = match delta_line {
                0 => at.character - column,
                _ => at.character,
            };
            data.push(SemanticToken {
                delta_line,
                delta_start,
                length: piece.encode_utf16().count() as u32,
                token_type,
                token_modifiers_bitset: 0,
            });
            (line, column) = (at.line, at.character);
        }
    }
    data
}
