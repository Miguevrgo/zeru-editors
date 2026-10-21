//! An error-tolerant parser for one file: it records the declarations, the
//! lexical scopes, the locals and every use of a name, with enough of each
//! expression to follow `a.b`, `A::B` and calls. Syntax errors are reported
//! once per mistake, and parsing resumes at the next statement or item.

use crate::lexer::{Kind, PRIMITIVES, Span, Token, lex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemKind {
    Fn,
    Struct,
    Enum,
    Trait,
    Const,
    Field,
    Method,
    Variant,
}

/// A declaration: an item of the file, or a field, method or variant of one.
/// `params` are a function's parameters as written, or a variant's payload
/// types, and `docs` the `//` lines right above it.
#[derive(Debug)]
pub struct Item {
    pub kind: ItemKind,
    pub name: String,
    pub name_span: Span,
    pub span: Span,
    pub owner: Option<usize>,
    pub is_pub: bool,
    pub decl: String,
    pub ty: Option<String>,
    pub params: Vec<String>,
    pub type_params: Vec<String>,
    pub has_self: bool,
    pub docs: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocalKind {
    Var,
    Param,
    Binding,
    TypeParam,
}

/// A name declared inside a scope, usable from `from` to the scope's end.
#[derive(Debug)]
pub struct Local {
    pub name: String,
    pub span: Span,
    pub kind: LocalKind,
    pub scope: usize,
    pub from: usize,
    pub decl: String,
    pub hint: Option<Hint>,
}

/// What the source says a local's type is: written out, the value it is
/// initialized with, an element of what a `for` walks, or a payload of the
/// variant a pattern names, matched against `subject`.
#[derive(Clone, Debug)]
pub enum Hint {
    Written(String),
    Value(Chain),
    Element(Chain),
    Payload {
        variant: usize,
        index: usize,
        subject: Chain,
    },
}

#[derive(Debug)]
pub struct Scope {
    pub span: Span,
    pub parent: Option<usize>,
}

/// `import a.b;` or `import a.b::{X, Y};`.
#[derive(Debug)]
pub struct Import {
    pub path: Vec<String>,
    pub symbols: Vec<String>,
}

#[derive(Debug)]
pub struct Use {
    pub span: Span,
    pub kind: UseKind,
}

#[derive(Debug, Clone)]
pub enum UseKind {
    Name,
    Type,
    Member { object: Chain, dot: usize },
    Path { qualifier: usize, sep: usize },
    Field { owner: usize },
    Module(usize),
    Symbol(usize),
}

/// The value an expression ends in, when it is a name followed by calls,
/// indexing, `?`/`!` or a struct literal's braces; or else the type its
/// source spells, as a literal's or a cast's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    pub at: Option<usize>,
    pub ops: Vec<Op>,
    pub ty: Option<String>,
}

impl Chain {
    fn typed(ty: &str) -> Self {
        Chain {
            ty: Some(ty.to_string()),
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Call,
    Index,
    Unwrap,
    Literal,
    Ref,
    RefVar,
    Deref,
}

#[derive(Debug, Default)]
pub struct Module {
    pub tokens: Vec<Token>,
    pub types: Vec<Span>,
    pub items: Vec<Item>,
    pub locals: Vec<Local>,
    pub scopes: Vec<Scope>,
    pub uses: Vec<Use>,
    pub calls: Vec<(usize, Vec<usize>)>,
    pub imports: Vec<Import>,
    pub errors: Vec<(Span, String)>,
}

pub fn parse(text: &str) -> Module {
    let tokens = lex(text);
    let mut parser = Parser {
        text,
        tokens: tokens
            .iter()
            .filter(|t| t.kind != Kind::Comment)
            .cloned()
            .chain([Token {
                kind: Kind::Op,
                span: Span::new(text.len(), text.len()),
            }])
            .collect(),
        pos: 0,
        module: Module {
            tokens,
            ..Module::default()
        },
        scope: None,
        panic: false,
        no_struct: false,
        depth: 0,
        nesting: 0,
    };
    parser.report_illegal();
    parser.program();
    parser.module
}

const MAX_NESTING: usize = 256;

const ITEM_STARTS: [&str; 7] = ["fn", "struct", "enum", "trait", "const", "import", "pub"];

const STATEMENT_STARTS: [&str; 10] = [
    "var", "fn", "const", "struct", "enum", "if", "while", "for", "return", "}",
];

#[derive(PartialEq, PartialOrd, Clone, Copy)]
enum Prec {
    Lowest,
    Assignment,
    LogicalOr,
    LogicalAnd,
    Equals,
    LessGreater,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    Shift,
    Sum,
    Product,
    Cast,
    Prefix,
    Call,
    Index,
}

fn precedence(op: &str) -> Prec {
    match op {
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | ">>=" | "<<=" | "+%="
        | "-%=" | "*%=" => Prec::Assignment,
        "||" => Prec::LogicalOr,
        "&&" => Prec::LogicalAnd,
        "|" => Prec::BitwiseOr,
        "^" => Prec::BitwiseXor,
        "&" => Prec::BitwiseAnd,
        "==" | "!=" => Prec::Equals,
        "<" | ">" | "<=" | ">=" => Prec::LessGreater,
        "<<" | ">>" => Prec::Shift,
        "+" | "-" | "+%" | "-%" => Prec::Sum,
        "*" | "/" | "%" | "*%" => Prec::Product,
        "as" => Prec::Cast,
        "(" => Prec::Call,
        "." | "[" | "?" | "!" => Prec::Index,
        _ => Prec::Lowest,
    }
}

/// Text with each run of whitespace made one space.
pub fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Parser<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    module: Module,
    scope: Option<usize>,
    panic: bool,
    no_struct: bool,
    depth: usize,
    nesting: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &str {
        self.nth(0)
    }

    fn nth(&self, ahead: usize) -> &str {
        let token = &self.tokens[(self.pos + ahead).min(self.tokens.len() - 1)];
        match token.kind {
            Kind::Str | Kind::Illegal(_) => "",
            _ => &self.text[token.span.start..token.span.end],
        }
    }

    fn kind(&self) -> &Kind {
        &self.tokens[self.pos].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn prev_end(&self) -> usize {
        match self.pos {
            0 => 0,
            at => self.tokens[at - 1].span.end,
        }
    }

    fn eof(&self) -> bool {
        self.pos == self.tokens.len() - 1
    }

    fn at(&self, text: &str) -> bool {
        self.peek() == text
    }

    fn at_ident(&self) -> bool {
        *self.kind() == Kind::Ident
    }

    fn bump(&mut self) -> String {
        let text = self.peek().to_string();
        if self.eof() {
            return text;
        }
        match text.as_str() {
            "{" => self.depth += 1,
            "}" => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        self.pos += 1;
        self.report_illegal();
        text
    }

    fn report_illegal(&mut self) {
        if let Kind::Illegal(message) = self.kind() {
            self.module.errors.push((self.span(), message.clone()));
            self.panic = true;
        }
    }

    fn eat(&mut self, text: &str) -> bool {
        let found = self.at(text);
        if found {
            self.bump();
        }
        found
    }

    fn expect(&mut self, text: &str) -> Option<()> {
        match self.eat(text) {
            true => Some(()),
            false => self.expected(&format!("'{text}'")),
        }
    }

    fn found(&self) -> String {
        match self.eof() {
            true => "end of file".into(),
            false => self.text[self.span().start..self.span().end].into(),
        }
    }

    fn expected<T>(&mut self, what: &str) -> Option<T> {
        let message = format!("Expected {what}, found '{}'", self.found());
        let at = self.pos.saturating_sub(1);
        self.error(message, self.tokens[at].span);
        None
    }

    fn error_here<T>(&mut self, message: impl Into<String>) -> Option<T> {
        self.error(message.into(), self.span());
        None
    }

    /// One error per mistake: nothing more until parsing recovers, nor next
    /// to a token the lexer already rejected.
    fn error(&mut self, message: String, span: Span) {
        let illegal = |at: usize| matches!(self.tokens[at].kind, Kind::Illegal(_));
        if self.panic || illegal(self.pos) || illegal(self.pos.saturating_sub(1)) {
            return;
        }
        self.panic = true;
        self.module.errors.push((span, message));
    }

    fn blamed(&self) -> bool {
        self.module
            .errors
            .last()
            .is_some_and(|(span, _)| *span == self.span())
    }

    fn ident(&mut self, what: &str) -> Option<Span> {
        if !self.at_ident() {
            return self.error_here(format!("Expected {what}, found '{}'", self.found()));
        }
        let span = self.span();
        self.bump();
        Some(span)
    }

    fn slice(&self, start: usize) -> String {
        collapse(&self.text[start..self.prev_end().max(start)])
    }

    /// The `//` lines right above the line `start` is on, up to a blank
    /// line or a rule of three or more dashes or other symbols.
    fn docs(&self, start: usize) -> String {
        let above = &self.text[..self.text[..start].rfind('\n').unwrap_or_default()];
        let rule = |line: &str| {
            let line = line.trim();
            line.len() >= 3 && !line.starts_with('`') && line.chars().all(|c| line.starts_with(c))
        };
        let mut lines: Vec<&str> = above
            .lines()
            .rev()
            .map_while(|line| line.trim().strip_prefix("//"))
            .map(|line| line.trim_start_matches('/'))
            .take_while(|line| !rule(line))
            .map(|line| line.strip_prefix(' ').unwrap_or(line))
            .collect();
        lines.reverse();
        lines.join("\n")
    }

    fn name(&self, span: Span) -> String {
        self.text[span.start..span.end].to_string()
    }

    /// Replace the next token by its first `len` bytes and the rest, as
    /// `>>` closes two generic lists.
    fn split(&mut self, len: usize) {
        let Span { start, end } = self.span();
        self.tokens[self.pos].span.end = start + len;
        let rest = Token {
            kind: Kind::Op,
            span: Span::new(start + len, end),
        };
        self.tokens.insert(self.pos + 1, rest);
    }

    fn nested<T>(&mut self, parse: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if self.nesting == MAX_NESTING {
            return self.error_here(format!("Code nests more than {MAX_NESTING} levels deep"));
        }
        self.nesting += 1;
        let parsed = parse(self);
        self.nesting -= 1;
        parsed
    }

    fn scoped<T>(&mut self, start: usize, parse: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let id = self.module.scopes.len();
        self.module.scopes.push(Scope {
            span: Span::new(start, start),
            parent: self.scope,
        });
        let outer = self.scope.replace(id);
        let parsed = parse(self);
        self.scope = outer;
        self.module.scopes[id].span.end = self.prev_end().max(start);
        parsed
    }

    fn add_use(&mut self, span: Span, kind: UseKind) -> usize {
        self.module.uses.push(Use { span, kind });
        self.module.uses.len() - 1
    }

    fn declare(&mut self, span: Span, kind: LocalKind, decl: String, hint: Option<Hint>) -> usize {
        self.module.locals.push(Local {
            name: self.name(span),
            span,
            kind,
            scope: self.scope.unwrap_or_default(),
            from: span.end,
            decl,
            hint,
        });
        self.module.locals.len() - 1
    }

    fn add_item(
        &mut self,
        kind: ItemKind,
        name: Span,
        start: usize,
        is_pub: bool,
        owner: Option<usize>,
    ) -> usize {
        self.module.items.push(Item {
            kind,
            name: self.name(name),
            name_span: name,
            span: Span::new(start, name.end),
            owner,
            is_pub,
            decl: self.slice(start),
            ty: None,
            params: Vec::new(),
            type_params: Vec::new(),
            has_self: false,
            docs: self.docs(start),
        });
        self.module.items.len() - 1
    }

    fn finish_item(&mut self, item: usize) {
        self.module.items[item].span.end = self.prev_end();
    }

    fn list(&mut self, close: &str, mut item: impl FnMut(&mut Self) -> Option<()>) -> Option<()> {
        while !self.eat(close) {
            item(self)?;
            if !self.at(close) {
                self.expect(",")?;
            }
        }
        Some(())
    }

    fn program(&mut self) {
        while !self.eof() {
            self.panic = false;
            let parsed = match self.peek() {
                ";" => {
                    self.bump();
                    continue;
                }
                "var" => self.error_here(
                    "Global variables ('var') are not allowed. Use 'const' for constants or move 'var' inside a function",
                ),
                _ => self.item(),
            };
            if parsed.is_none() {
                self.skip_item();
            }
        }
    }

    fn skip_item(&mut self) {
        self.panic = false;
        let mut blamed = self.blamed();
        while !self.eof() {
            if !blamed && self.depth == 0 && ITEM_STARTS.contains(&self.peek()) {
                return;
            }
            blamed = false;
            let token = self.bump();
            if self.depth == 0 && (token == "}" || token == ";") {
                return;
            }
        }
    }

    fn item(&mut self) -> Option<()> {
        let start = self.span().start;
        let is_pub = self.eat("pub");
        match self.peek() {
            "fn" => self.function(start, is_pub, None),
            "struct" => self.structure(start, is_pub),
            "enum" => self.enumeration(start, is_pub),
            "trait" => self.trait_(start, is_pub),
            "const" => self.constant(start, is_pub),
            "import" if !is_pub => self.import(),
            _ if is_pub => self.error_here("'pub' goes before fn, struct, enum, trait or const"),
            token => {
                let message = format!(
                    "Unexpected '{token}' at top level, expected fn, struct, enum, trait, const or import"
                );
                self.error_here(message)
            }
        }
    }

    fn import(&mut self) -> Option<()> {
        if self.module.items.iter().any(|item| item.owner.is_none()) {
            self.error_here::<()>("Imports must come before any other code");
        }
        self.bump();
        let mut path = Vec::new();
        let mut span = self.ident("a name")?;
        path.push(self.name(span));
        while self.eat(".") {
            span = self.ident("a name")?;
            path.push(self.name(span));
        }
        let import = self.module.imports.len();
        self.module.imports.push(Import {
            path,
            symbols: Vec::new(),
        });
        self.add_use(span, UseKind::Module(import));
        if self.eat("::") {
            self.expect("{")?;
            self.list("}", |p| {
                let symbol = p.ident("imported name")?;
                p.add_use(symbol, UseKind::Symbol(import));
                let name = p.name(symbol);
                p.module.imports[import].symbols.push(name);
                Some(())
            })?;
        }
        self.expect(";")
    }

    fn function(&mut self, start: usize, is_pub: bool, owner: Option<usize>) -> Option<()> {
        self.bump();
        let name = self.ident("a name")?;
        let kind = match owner {
            Some(_) => ItemKind::Method,
            None => ItemKind::Fn,
        };
        let item = self.add_item(kind, name, start, is_pub, owner);
        let in_trait = owner.is_some_and(|o| self.module.items[o].kind == ItemKind::Trait);
        let parsed = self.scoped(start, |p| {
            if !in_trait {
                p.type_params(item)?;
            }
            p.expect("(")?;
            let mut params = Vec::new();
            let mut has_self = false;
            p.list(")", |p| {
                let start = p.span().start;
                let is_var = p.eat("var");
                if p.at("self") {
                    let span = p.span();
                    p.bump();
                    let owner = owner.map(|o| p.module.items[o].name.clone());
                    let decl = match is_var {
                        true => "var self".to_string(),
                        false => "self".to_string(),
                    };
                    p.declare(span, LocalKind::Param, decl, owner.map(Hint::Written));
                    has_self = true;
                    return Some(());
                }
                let name = p.ident("parameter name")?;
                p.expect(":")?;
                let ty = p.ty()?;
                let decl = p.slice(start);
                p.declare(
                    name,
                    LocalKind::Param,
                    decl.clone(),
                    Some(Hint::Written(ty)),
                );
                params.push(decl);
                Some(())
            })?;
            let body_starts: &[&str] = match in_trait {
                true => &[";", "}", "fn"],
                false => &["{"],
            };
            let ty = match body_starts.contains(&p.peek()) {
                true => None,
                false if p.eat("!") => Some("!".to_string()),
                false => Some(p.ty()?),
            };
            let header = p.slice(start);
            let entry = &mut p.module.items[item];
            (entry.decl, entry.ty, entry.params, entry.has_self) = (header, ty, params, has_self);
            match in_trait {
                true => p.expect(";"),
                false => p.block(),
            }
        });
        self.finish_item(item);
        parsed
    }

    fn type_params(&mut self, item: usize) -> Option<()> {
        if !self.eat("<") {
            return Some(());
        }
        self.list(">", |p| {
            let start = p.span().start;
            let name = p.ident("type parameter name")?;
            if p.eat(":") {
                p.path(UseKind::Type)?;
            }
            let decl = p.slice(start);
            p.declare(name, LocalKind::TypeParam, decl, None);
            let name = p.name(name);
            p.module.items[item].type_params.push(name);
            Some(())
        })
    }

    fn structure(&mut self, start: usize, is_pub: bool) -> Option<()> {
        self.bump();
        let name = self.ident("a name")?;
        let item = self.add_item(ItemKind::Struct, name, start, is_pub, None);
        let parsed = self.scoped(start, |p| {
            p.type_params(item)?;
            p.module.items[item].decl = p.slice(start);
            p.expect("{")?;
            let depth = p.depth;
            let mut seen_method = false;
            while !p.at("}") && !p.eof() {
                if p.at("fn") || p.at("pub") {
                    seen_method = true;
                    let start = p.span().start;
                    let is_pub = p.eat("pub");
                    if !p.at("fn") {
                        return p.expected("'fn'");
                    }
                    if p.function(start, is_pub, Some(item)).is_none() {
                        p.panic = false;
                        while !p.eof()
                            && !(p.depth == depth && ["fn", "pub", "}"].contains(&p.peek()))
                        {
                            p.bump();
                        }
                    }
                    continue;
                }
                if seen_method {
                    let name = &p.module.items[item].name;
                    let message =
                        format!("Struct '{name}': Fields must be declared before methods");
                    p.error(message, p.tokens[p.pos - 1].span);
                    return None;
                }
                if p.field(item).is_none() {
                    while !p.eof() && ![",", "}"].contains(&p.peek()) {
                        p.bump();
                    }
                    p.eat(",");
                    continue;
                }
                if p.at("}") {
                    break;
                }
                if !p.eat(",") {
                    p.expected::<()>("','");
                    p.panic = false;
                }
            }
            p.expect("}")
        });
        self.finish_item(item);
        parsed
    }

    fn field(&mut self, owner: usize) -> Option<()> {
        let name = self.ident("field name")?;
        let item = self.add_item(ItemKind::Field, name, name.start, false, Some(owner));
        self.expect(":")?;
        let ty = Some(self.ty()?);
        let decl = self.slice(name.start);
        let entry = &mut self.module.items[item];
        (entry.ty, entry.decl) = (ty, decl);
        self.finish_item(item);
        Some(())
    }

    fn enumeration(&mut self, start: usize, is_pub: bool) -> Option<()> {
        self.bump();
        let name = self.ident("a name")?;
        let item = self.add_item(ItemKind::Enum, name, start, is_pub, None);
        self.expect("{")?;
        let parsed = self.list("}", |p| {
            let name = p.ident("enum variant name")?;
            let variant = p.add_item(ItemKind::Variant, name, name.start, false, Some(item));
            if p.eat("(") {
                p.list(")", |p| {
                    let ty = p.ty()?;
                    p.module.items[variant].params.push(ty);
                    Some(())
                })?;
            }
            p.module.items[variant].decl = p.slice(name.start);
            p.finish_item(variant);
            Some(())
        });
        self.finish_item(item);
        parsed
    }

    fn trait_(&mut self, start: usize, is_pub: bool) -> Option<()> {
        self.bump();
        let name = self.ident("a name")?;
        let item = self.add_item(ItemKind::Trait, name, start, is_pub, None);
        self.expect("{")?;
        let parsed = (|| {
            while !self.at("}") && !self.eof() {
                if !self.at("fn") {
                    return self.expected("'fn'");
                }
                let start = self.span().start;
                self.function(start, false, Some(item))?;
            }
            self.expect("}")
        })();
        self.finish_item(item);
        parsed
    }

    fn constant(&mut self, start: usize, is_pub: bool) -> Option<()> {
        self.bump();
        let name = self.ident("a name")?;
        let item = self.add_item(ItemKind::Const, name, start, is_pub, None);
        if self.eat(":") {
            let ty = self.annotation()?;
            self.module.items[item].ty = Some(ty);
        }
        self.module.items[item].decl = self.slice(start);
        self.expect("=")?;
        self.expression()?;
        self.expect(";")?;
        self.finish_item(item);
        Some(())
    }

    fn block(&mut self) -> Option<()> {
        let start = self.span().start;
        self.expect("{")?;
        let depth = self.depth;
        self.scoped(start, |p| {
            while !p.at("}") && !p.eof() {
                p.panic = false;
                if p.statement().is_none() {
                    p.synchronize(depth);
                }
            }
            let reported = p
                .module
                .errors
                .last()
                .is_some_and(|(span, _)| span.end >= p.span().start);
            if !p.eat("}") && !reported {
                p.error_here::<()>("Expected '}', found 'end of file'");
            }
            Some(())
        })
    }

    fn synchronize(&mut self, depth: usize) {
        self.panic = false;
        let mut blamed = self.blamed() && !self.at("}");
        while !self.eof() {
            if !blamed && self.depth == depth && STATEMENT_STARTS.contains(&self.peek()) {
                return;
            }
            blamed = false;
            if self.bump() == ";" && self.depth == depth {
                return;
            }
        }
    }

    fn statement(&mut self) -> Option<()> {
        self.nested(|p| {
            let start = p.span().start;
            match p.peek() {
                "var" | "const" => p.var(),
                "return" => {
                    p.bump();
                    if !p.eat(";") {
                        p.expression()?;
                        p.expect(";")?;
                    }
                    Some(())
                }
                "fn" => p.function(start, false, None),
                "struct" => p.structure(start, false),
                "enum" => p.enumeration(start, false),
                "if" => p.if_(),
                "while" => {
                    p.bump();
                    p.condition()?;
                    p.block()
                }
                "for" => p.for_(),
                "break" | "continue" => {
                    p.bump();
                    p.expect(";")
                }
                "{" => p.block(),
                "import" => p.error_here("Imports must come before any other code"),
                "match" => {
                    let chain = p.match_()?;
                    if [".", "?", "!"].contains(&p.peek()) {
                        p.operators(chain, Prec::Lowest)?;
                        p.expect(";")?;
                    }
                    Some(())
                }
                _ => {
                    p.expression()?;
                    p.expect(";")
                }
            }
        })
    }

    fn var(&mut self) -> Option<()> {
        let start = self.span().start;
        self.bump();
        let name = self.ident("a name")?;
        let local = self.declare(name, LocalKind::Var, String::new(), None);
        self.module.locals[local].from = usize::MAX;
        let parsed = (|| {
            if self.eat(":") {
                let ty = self.annotation()?;
                self.module.locals[local].hint = Some(Hint::Written(ty));
            }
            self.module.locals[local].decl = self.slice(start);
            self.expect("=")?;
            let value = self.expression()?;
            let entry = &mut self.module.locals[local];
            if entry.hint.is_none() && (value.at.is_some() || value.ty.is_some()) {
                entry.hint = Some(Hint::Value(value));
            }
            self.expect(";")
        })();
        if self.module.locals[local].decl.is_empty() {
            self.module.locals[local].decl = self.slice(start);
        }
        self.module.locals[local].from = self.prev_end();
        parsed
    }

    /// The type of a `var` or `const`, whose `!` the lexer may have joined to
    /// the `=` after it, as in `i32!= 3`.
    fn annotation(&mut self) -> Option<String> {
        let start = self.span().start;
        self.ty()?;
        if self.at("!=") && self.span().start == self.prev_end() {
            self.split(1);
            self.bump();
        }
        Some(self.slice(start))
    }

    fn if_(&mut self) -> Option<()> {
        self.bump();
        self.condition()?;
        self.block()?;
        if self.eat("else") {
            return match self.at("if") {
                true => self.if_(),
                false => self.block(),
            };
        }
        Some(())
    }

    fn for_(&mut self) -> Option<()> {
        let start = self.span().start;
        self.bump();
        self.scoped(start, |p| {
            let mut name = p.ident("a name")?;
            if p.eat(",") {
                let index = p.declare(name, LocalKind::Binding, p.name(name), None);
                p.module.locals[index].hint = Some(Hint::Written("usize".into()));
                name = p.ident("a name")?;
            }
            p.expect("in")?;
            let walked = p.condition()?;
            let hint = match p.eat("..") {
                true => {
                    p.condition()?;
                    Hint::Written("i32".into())
                }
                false => Hint::Element(walked),
            };
            let local = p.declare(name, LocalKind::Binding, p.name(name), Some(hint));
            p.module.locals[local].from = p.prev_end();
            p.block()
        })
    }

    fn ty(&mut self) -> Option<String> {
        let from = self.prev_end();
        let start = self.span().start;
        let parsed = self.ty_inner();
        self.module.types.push(Span::new(from, self.span().start));
        parsed?;
        Some(self.slice(start))
    }

    fn ty_inner(&mut self) -> Option<()> {
        self.nested(|p| {
            match p.peek() {
                "str" => {
                    p.bump();
                }
                "*" => {
                    p.bump();
                    p.ty_inner()?;
                }
                "&" => {
                    p.bump();
                    if p.eat("[") {
                        p.ty_inner()?;
                        p.expect("]")?;
                    } else {
                        p.eat("var");
                        p.ty_inner()?;
                    }
                }
                "(" => {
                    p.bump();
                    p.list(")", Self::ty_inner)?;
                }
                _ if p.at_ident() => {
                    let primitive = PRIMITIVES.contains(&p.peek());
                    p.path(UseKind::Type)?;
                    if p.at("<") && !primitive {
                        p.generic_args()?;
                    }
                }
                _ => return p.error_here("Type identifier expected"),
            }
            if !p.eat("?") {
                p.eat("!");
            }
            Some(())
        })
    }

    fn generic_args(&mut self) -> Option<()> {
        self.bump();
        while !self.eat(">") {
            match self.kind() {
                Kind::Int => {
                    self.bump();
                }
                _ => self.ty_inner()?,
            }
            if [">>", ">=", ">>="].contains(&self.peek()) {
                self.split(1);
            }
            if !self.at(">") && !self.eat(",") {
                return self.expected("',' or '>' in generic type");
            }
        }
        Some(())
    }

    /// `a::b::c`, each name a use; the last one's index.
    fn path(&mut self, first: UseKind) -> Option<usize> {
        let span = self.ident("a name")?;
        let mut last = self.add_use(span, first);
        while self.at("::") {
            let sep = self.span().end;
            self.bump();
            let qualifier = last;
            if !self.at_ident() {
                self.add_use(Span::new(sep, sep), UseKind::Path { qualifier, sep });
                return self.expected("a name");
            }
            let span = self.ident("a name")?;
            last = self.add_use(span, UseKind::Path { qualifier, sep });
        }
        Some(last)
    }

    fn condition(&mut self) -> Option<Chain> {
        let saved = std::mem::replace(&mut self.no_struct, true);
        let parsed = self.expression();
        self.no_struct = saved;
        parsed
    }

    fn delimited<T>(&mut self, parse: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.no_struct, false);
        let parsed = parse(self);
        self.no_struct = saved;
        parsed
    }

    fn expression(&mut self) -> Option<Chain> {
        self.expr(Prec::Lowest)
    }

    fn expr(&mut self, prec: Prec) -> Option<Chain> {
        self.nested(|p| {
            let operand = p.operand()?;
            p.operators(operand, prec)
        })
    }

    fn operators(&mut self, mut left: Chain, prec: Prec) -> Option<Chain> {
        let mut compared = false;
        while prec < precedence(self.peek()) {
            let next = precedence(self.peek());
            let comparison = matches!(next, Prec::Equals | Prec::LessGreater);
            if comparison && (compared || prec == Prec::Equals) {
                return self.error_here("Comparison operators cannot be chained");
            }
            compared = comparison;
            left = self.infix(left)?;
        }
        Some(left)
    }

    fn operand(&mut self) -> Option<Chain> {
        match self.kind() {
            Kind::Int | Kind::Float | Kind::Str => {
                let ty = match self.kind() {
                    Kind::Float => "f64",
                    Kind::Str if self.text[self.span().start..].starts_with('"') => "str",
                    _ => "i32",
                };
                self.bump();
                return Some(Chain::typed(ty));
            }
            Kind::Ident => return self.path_or_literal(),
            _ => {}
        }
        match self.peek() {
            "true" | "false" => {
                self.bump();
                Some(Chain::typed("bool"))
            }
            "None" => {
                self.bump();
                Some(Chain::default())
            }
            "self" => {
                let at = self.add_use(self.span(), UseKind::Name);
                self.bump();
                Some(Chain {
                    at: Some(at),
                    ops: Vec::new(),
                    ty: None,
                })
            }
            "[" => {
                self.bump();
                self.delimited(Self::array)?;
                Some(Chain::default())
            }
            "(" => self.grouped(),
            "-" | "!" => {
                self.bump();
                self.expr(Prec::Prefix)
            }
            "&" | "*" => {
                let op = match self.bump().as_str() {
                    "*" => Op::Deref,
                    _ if self.eat("var") => Op::RefVar,
                    _ => Op::Ref,
                };
                let mut chain = self.expr(Prec::Prefix)?;
                chain.ops.push(op);
                Some(chain)
            }
            "match" => self.match_(),
            "asm" => self.asm(),
            _ => self.error_here(format!("Expected an expression, found '{}'", self.found())),
        }
    }

    fn path_or_literal(&mut self) -> Option<Chain> {
        let last = self.path(UseKind::Name)?;
        if !self.no_struct && self.at("{") {
            self.bump();
            self.delimited(|p| {
                p.list("}", |p| {
                    let field = p.ident("field name")?;
                    p.add_use(field, UseKind::Field { owner: last });
                    p.expect(":")?;
                    p.expression().map(drop)
                })
            })?;
            return Some(Chain {
                at: Some(last),
                ops: vec![Op::Literal],
                ty: None,
            });
        }
        self.explicit_call(Chain {
            at: Some(last),
            ops: Vec::new(),
            ty: None,
        })
    }

    /// `function<T, ..>(..)`, when what follows the `<` reads as types
    /// closed by `>` and then `(`; otherwise the `<` is a comparison.
    fn explicit_call(&mut self, mut chain: Chain) -> Option<Chain> {
        if !self.at("<") || !self.generic_call_ahead() {
            return Some(chain);
        }
        self.generic_args()?;
        self.expect("(")?;
        self.delimited(|p| p.list(")", |p| p.expression().map(drop)))?;
        chain.ops.push(Op::Call);
        Some(chain)
    }

    fn generic_call_ahead(&self) -> bool {
        let mut depth = 0;
        for (at, token) in self.tokens[self.pos..].iter().enumerate() {
            let text = &self.text[token.span.start..token.span.end];
            match text {
                "<" => depth += 1,
                ">" => depth -= 1,
                ">>" => depth -= 2,
                "," | "::" | "&" | "*" | "?" | "!" | "(" | ")" | "[" | "]" | "var" | "str" => {}
                _ if token.kind == Kind::Ident => {}
                _ => return false,
            }
            if depth <= 0 {
                return depth == 0 && self.nth(at + 1) == "(";
            }
        }
        false
    }

    fn infix(&mut self, mut left: Chain) -> Option<Chain> {
        let op = self.bump();
        match op.as_str() {
            "[" => {
                self.delimited(Self::expression)?;
                self.expect("]")?;
                left.ops.push(Op::Index);
                Some(left)
            }
            "(" => {
                let mut args = Vec::new();
                self.delimited(|p| {
                    p.list(")", |p| {
                        args.push(p.span().start);
                        p.expression().map(drop)
                    })
                })?;
                if let (Some(callee), true) = (left.at, left.ops.is_empty()) {
                    self.module.calls.push((callee, args));
                }
                left.ops.push(Op::Call);
                Some(left)
            }
            "." => {
                let dot = self.prev_end();
                if *self.kind() == Kind::Int {
                    self.bump();
                    return Some(Chain::default());
                }
                if !self.at_ident() {
                    let object = left;
                    self.add_use(Span::new(dot, dot), UseKind::Member { object, dot });
                    return self.error_here("Expected a field name after '.'");
                }
                let span = self.span();
                self.bump();
                let at = self.add_use(span, UseKind::Member { object: left, dot });
                self.explicit_call(Chain {
                    at: Some(at),
                    ops: Vec::new(),
                    ty: None,
                })
            }
            "?" | "!" => {
                left.ops.push(Op::Unwrap);
                Some(left)
            }
            "as" => Some(Chain::typed(&self.ty()?)),
            _ => {
                let prec = precedence(&op);
                let right = match prec == Prec::Assignment {
                    true => self.expr(Prec::Lowest),
                    false => self.expr(prec),
                }?;
                Some(match prec {
                    Prec::Assignment => Chain::default(),
                    Prec::LogicalOr | Prec::LogicalAnd | Prec::Equals | Prec::LessGreater => {
                        Chain::typed("bool")
                    }
                    _ if left.at.is_none() && right.at.is_some() => right,
                    _ => left,
                })
            }
        }
    }

    fn grouped(&mut self) -> Option<Chain> {
        self.bump();
        self.delimited(|p| {
            if p.eat(")") {
                return Some(Chain::default());
            }
            let first = p.expression()?;
            if !p.eat(",") {
                p.expect(")")?;
                return Some(first);
            }
            p.list(")", |p| p.expression().map(drop))?;
            Some(Chain::default())
        })
    }

    fn array(&mut self) -> Option<()> {
        if self.eat("]") {
            return Some(());
        }
        self.expression()?;
        if self.eat(";") {
            self.expression()?;
            return self.expect("]");
        }
        match self.eat(",") {
            true => self.list("]", |p| p.expression().map(drop)),
            false => self.expect("]"),
        }
    }

    fn match_(&mut self) -> Option<Chain> {
        self.bump();
        let subject = self.condition()?;
        self.expect("{")?;
        while !self.at("}") && !self.eof() {
            let start = self.span().start;
            self.scoped(start, |p| {
                if !p.eat("default") {
                    p.pattern(&subject, None)?;
                }
                p.expect("=>")?;
                let block = p.at("{");
                match block {
                    true => p.block()?,
                    false => p.expression().map(drop)?,
                }
                match block || p.at("}") {
                    true => {
                        p.eat(",");
                        Some(())
                    }
                    false => p.expect(","),
                }
            })?;
        }
        self.expect("}")?;
        Some(Chain::default())
    }

    /// Alternatives separated by `|`, matched against `subject`. Inside a
    /// payload, `slot` is the variant's use and the payload's index.
    fn pattern(&mut self, subject: &Chain, slot: Option<(usize, usize)>) -> Option<()> {
        self.nested(|p| {
            p.alternative(subject, slot)?;
            while p.eat("|") {
                p.alternative(subject, slot)?;
            }
            Some(())
        })
    }

    /// `E::V(p, q)` or `Some(p)`, a name bound inside a payload, or else a
    /// constant expression stopping before `|`.
    fn alternative(&mut self, subject: &Chain, slot: Option<(usize, usize)>) -> Option<()> {
        if !self.at_ident() {
            return self.expr(Prec::BitwiseOr).map(drop);
        }
        let mut ahead = 1;
        while self.nth(ahead) == "::"
            && self
                .tokens
                .get(self.pos + ahead + 1)
                .is_some_and(|t| t.kind == Kind::Ident)
        {
            ahead += 2;
        }
        let after = self.nth(ahead);
        if after == "(" {
            let variant = self.path(UseKind::Name)?;
            self.bump();
            let inner = match slot {
                Some(_) => Chain::default(),
                None => subject.clone(),
            };
            let mut index = 0;
            return self.list(")", |p| {
                p.pattern(&inner, Some((variant, index)))?;
                index += 1;
                Some(())
            });
        }
        if let Some((variant, index)) = slot
            && ahead == 1
            && [",", ")", "|"].contains(&after)
        {
            let span = self.span();
            self.bump();
            if self.name(span) != "_" {
                let subject = subject.clone();
                let hint = Hint::Payload {
                    variant,
                    index,
                    subject,
                };
                self.declare(span, LocalKind::Binding, self.name(span), Some(hint));
            }
            return Some(());
        }
        self.expr(Prec::BitwiseOr).map(drop)
    }

    fn asm(&mut self) -> Option<Chain> {
        self.bump();
        self.eat("volatile");
        self.expect("(")?;
        if *self.kind() != Kind::Str {
            return self.error_here(format!(
                "Expected assembly template string, found '{}'",
                self.found()
            ));
        }
        while !self.at(")") && !self.eof() {
            if self.eat("(") {
                self.expression()?;
                self.expect(")")?;
            } else {
                self.bump();
            }
        }
        self.expect(")")?;
        Some(Chain::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "import std.math;
import geom.shapes::{Rect};

struct Point<T> {
    x: i32,
    y: Vec<Vec<i32>>,

    pub fn sum(self, k: i32) i32 {
        return self.x + k;
    }
}

enum Shape { Circle(f64), Empty }

const LIMIT: i32!= 3;

fn main() {
    var p = Point { x: 1, y: Vec.new() };
    var v: Vec<i64> = Vec.new();
    var s = Shape::Circle(1.0);
    match s {
        Shape::Circle(r) => { p.sum(r); }
        default => {}
    }
    for i in 0..10 { v.push(i as i64); }
    p.y[0].len();
}
";

    fn texts<'a>(uses: impl Iterator<Item = &'a Use>) -> Vec<&'static str> {
        uses.map(|u| &SOURCE[u.span.start..u.span.end]).collect()
    }

    #[test]
    fn declarations_and_scopes() {
        let module = parse(SOURCE);
        assert!(module.errors.is_empty(), "{:?}", module.errors);
        let items: Vec<(ItemKind, &str)> = module
            .items
            .iter()
            .map(|i| (i.kind, i.name.as_str()))
            .collect();
        assert_eq!(
            items,
            [
                (ItemKind::Struct, "Point"),
                (ItemKind::Field, "x"),
                (ItemKind::Field, "y"),
                (ItemKind::Method, "sum"),
                (ItemKind::Enum, "Shape"),
                (ItemKind::Variant, "Circle"),
                (ItemKind::Variant, "Empty"),
                (ItemKind::Const, "LIMIT"),
                (ItemKind::Fn, "main"),
            ]
        );
        assert_eq!(module.items[3].decl, "pub fn sum(self, k: i32) i32");
        assert_eq!(module.items[3].params, ["k: i32"]);
        assert_eq!(module.items[2].ty.as_deref(), Some("Vec<Vec<i32>>"));
        assert_eq!(module.items[5].decl, "Circle(f64)");
        assert_eq!(module.items[7].decl, "const LIMIT: i32!");
        let locals: Vec<&str> = module.locals.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(locals, ["T", "self", "k", "p", "v", "s", "r", "i"]);
        assert_eq!(module.locals[4].decl, "var v: Vec<i64>");
        assert!(
            matches!(module.locals[3].hint, Some(Hint::Value(ref c)) if c.ops == [Op::Literal])
        );
        let imports: Vec<&Vec<String>> = module.imports.iter().map(|i| &i.path).collect();
        assert_eq!(
            imports,
            [
                &vec!["std".to_string(), "math".into()],
                &vec!["geom".to_string(), "shapes".into()]
            ]
        );
    }

    #[test]
    fn uses_follow_members_and_paths() {
        let module = parse(SOURCE);
        let members = module
            .uses
            .iter()
            .filter(|u| matches!(u.kind, UseKind::Member { .. }));
        assert_eq!(
            texts(members),
            ["x", "new", "new", "sum", "push", "y", "len"]
        );
        let len = module
            .uses
            .iter()
            .find(|u| &SOURCE[u.span.start..u.span.end] == "len")
            .unwrap();
        let UseKind::Member { object, .. } = &len.kind else {
            panic!()
        };
        assert_eq!(object.ops, [Op::Index]);
        let paths = module
            .uses
            .iter()
            .filter(|u| matches!(u.kind, UseKind::Path { .. }));
        assert_eq!(texts(paths), ["Circle", "Circle"]);
    }

    #[test]
    fn errors_are_reported_once_and_parsing_resumes() {
        let module = parse(
            "fn main() {\n    var a = ;\n    var b: i32 = 1\n    var c = 2;\n    p.\n}\nfn after() {}\n",
        );
        let messages: Vec<&str> = module.errors.iter().map(|(_, m)| m.as_str()).collect();
        assert_eq!(
            messages,
            [
                "Expected an expression, found ';'",
                "Expected ';', found 'var'",
                "Expected a field name after '.'"
            ]
        );
        assert!(module.items.iter().any(|i| i.name == "after"));
        let empty = module.uses.iter().find(|u| u.span.start == u.span.end);
        assert!(matches!(
            empty.map(|u| &u.kind),
            Some(UseKind::Member { .. })
        ));
    }

    #[test]
    fn the_standard_library_parses_cleanly() {
        let zeru = crate::check::find_zeru(None);
        let std = crate::check::find_std(zeru.as_deref()).expect("the Zeru standard library");
        let mut dirs = vec![std];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "zr") {
                    let errors = parse(&std::fs::read_to_string(&path).unwrap()).errors;
                    assert!(errors.is_empty(), "{}: {errors:?}", path.display());
                }
            }
        }
    }
}
