//! The program: the open files, every module they import, the prelude and
//! the compiler's built-ins, each parsed on its own, and what each name in
//! them refers to. Types are only the ones written in the source.

use std::path::{Path, PathBuf};

use crate::lexer::{PRIMITIVES, Span, lex};
use crate::parser::{Chain, Hint, ItemKind, LocalKind, Module, Op, UseKind, parse};

pub struct File {
    pub path: PathBuf,
    pub text: String,
    pub module: Module,
    pub std: bool,
    pub imports: Vec<Option<usize>>,
}

pub struct Program {
    pub files: Vec<File>,
    pub prelude: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Def {
    Local(usize, usize),
    Item(usize, usize),
    Module(usize),
}

/// What a value is known to be: a type written in a file, or a struct or
/// enum named as such.
#[derive(Debug)]
pub enum Ty {
    Value(usize, String),
    Static(usize, usize),
}

/// The file of the program that declares what the compiler builds in: `Vec`,
/// `String`, the methods of `str`, slices, integers, `T?` and `T!` as `Str`,
/// `Slice<T>`, `Int<T>`, `Optional<T, U>` (`U` is `T` read through a
/// reference) and `Result<T>`, the print functions, `assert` and `replace`.
pub const BUILTINS: usize = 0;

const INTEGERS: [&str; 10] = [
    "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize",
];

/// The built-in struct a written type is, and its type arguments.
fn builtin(ty: &str) -> Option<(&'static str, Vec<&str>)> {
    let ty = strip(ty);
    if let Some(inner) = ty.strip_suffix('?') {
        return Some(("Optional", vec![inner, strip(inner)]));
    }
    if let Some(inner) = ty.strip_suffix('!') {
        return Some(("Result", vec![inner]));
    }
    if let Some(inner) = ty.strip_prefix('[').and_then(|ty| ty.strip_suffix(']')) {
        return Some(("Slice", vec![inner]));
    }
    if INTEGERS.contains(&ty) {
        return Some(("Int", vec![ty]));
    }
    (ty == "str").then(|| ("Str", Vec::new()))
}

/// `ty` with each type parameter in `params` replaced by its argument.
fn substitute(ty: &str, params: &[String], args: &[&str]) -> String {
    let mut found = String::new();
    let mut last = 0;
    for token in lex(ty) {
        let name = &ty[token.span.start..token.span.end];
        if let Some(arg) = params
            .iter()
            .position(|p| p == name)
            .and_then(|at| args.get(at))
        {
            found += &ty[last..token.span.start];
            found += arg;
            last = token.span.end;
        }
    }
    found + &ty[last..]
}

impl Program {
    /// Parse `roots`, the modules they import, transitively, and the prelude
    /// in `std`, reading each file with `read`. Imports resolve against the
    /// directory of the root that reached them, as the compiler's do.
    pub fn load(
        roots: &[PathBuf],
        std: Option<&Path>,
        read: impl Fn(&Path) -> Option<String>,
    ) -> Self {
        let text = include_str!("builtins.zr");
        let builtins = File {
            path: PathBuf::new(),
            text: text.to_string(),
            module: parse(text),
            std: true,
            imports: Vec::new(),
        };
        let mut program = Program {
            files: vec![builtins],
            prelude: None,
        };
        let mut queue = Vec::new();
        if let Some(std) = std {
            let path = std.join("builtin.zr");
            program.prelude = program.add(path, &read, true);
            queue.extend(program.prelude.map(|at| (at, std.to_path_buf())));
            for path in modules(std) {
                if let Some(at) = program.add(path, &read, true) {
                    queue.push((at, std.to_path_buf()));
                }
            }
        }
        for root in roots {
            let dir = root.parent().unwrap_or(Path::new("")).to_path_buf();
            if let Some(at) = program.add(root.clone(), &read, false) {
                queue.push((at, dir));
            }
        }
        while let Some((at, dir)) = queue.pop() {
            for k in 0..program.files[at].module.imports.len() {
                let path = &program.files[at].module.imports[k].path;
                let (base, rest) = match (path.split_first(), std) {
                    (Some((first, rest)), Some(std)) if first == "std" => (std, rest),
                    _ => (dir.as_path(), &path[..]),
                };
                let file = base.join(rest.join("/")).with_extension("zr");
                let target = program.find(&file).or_else(|| {
                    let std = std.is_some_and(|std| file.starts_with(std));
                    let added = program.add(file, &read, std)?;
                    queue.push((added, dir.clone()));
                    Some(added)
                });
                program.files[at].imports.push(target);
            }
        }
        program
    }

    fn add(
        &mut self,
        path: PathBuf,
        read: impl Fn(&Path) -> Option<String>,
        std: bool,
    ) -> Option<usize> {
        if let Some(at) = self.find(&path) {
            return Some(at);
        }
        let text = read(&path)?;
        self.files.push(File {
            module: parse(&text),
            path,
            text,
            std,
            imports: Vec::new(),
        });
        Some(self.files.len() - 1)
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        self.files.iter().position(|file| file.path == path)
    }

    pub fn text(&self, file: usize, span: Span) -> &str {
        &self.files[file].text[span.start..span.end]
    }

    /// The item of `file` called `name`, not a field, method or variant.
    pub fn top_item(&self, file: usize, name: &str) -> Option<usize> {
        let items = &self.files[file].module.items;
        items
            .iter()
            .position(|item| item.owner.is_none() && item.name == name)
    }

    pub fn children(&self, file: usize, owner: usize) -> impl Iterator<Item = usize> + '_ {
        let items = &self.files[file].module.items;
        (0..items.len()).filter(move |&at| items[at].owner == Some(owner))
    }

    fn child(&self, file: usize, owner: usize, name: &str) -> Option<Def> {
        let mut children = self.children(file, owner);
        let found = children.find(|&at| self.files[file].module.items[at].name == name)?;
        Some(Def::Item(file, found))
    }

    /// The member `name` of `owner`: its method when `called`, else its
    /// field, as a struct may have both.
    fn member(&self, file: usize, owner: usize, name: &str, called: bool) -> Option<Def> {
        let items = &self.files[file].module.items;
        let named: Vec<usize> = self
            .children(file, owner)
            .filter(|&at| items[at].name == name)
            .collect();
        let wanted = named
            .iter()
            .find(|&&at| (items[at].kind == ItemKind::Method) == called);
        Some(Def::Item(file, *wanted.or(named.first())?))
    }

    /// The innermost scope of `file` around `offset`.
    pub fn scope_at(&self, file: usize, offset: usize) -> Option<usize> {
        let scopes = &self.files[file].module.scopes;
        (0..scopes.len())
            .filter(|&at| scopes[at].span.start < offset && offset <= scopes[at].span.end)
            .max_by_key(|&at| scopes[at].span.start)
    }

    /// The locals visible at `offset`, innermost and latest first.
    pub fn locals_at(&self, file: usize, offset: usize) -> Vec<usize> {
        let module = &self.files[file].module;
        let mut found = Vec::new();
        let mut scope = self.scope_at(file, offset);
        while let Some(at) = scope {
            let mut here: Vec<usize> = (0..module.locals.len())
                .filter(|&l| module.locals[l].scope == at && module.locals[l].from <= offset)
                .collect();
            here.sort_by_key(|&l| std::cmp::Reverse(module.locals[l].from));
            found.extend(here);
            scope = module.scopes[at].parent;
        }
        found
    }

    /// What `name` means at `offset` in `file`: a local, else a global.
    /// In a type only type parameters are locals, elsewhere they never are.
    fn lookup(&self, file: usize, name: &str, offset: usize, in_type: bool) -> Option<Def> {
        let locals = &self.files[file].module.locals;
        let local = self.locals_at(file, offset).into_iter().find(|&l| {
            locals[l].name == name && (locals[l].kind == LocalKind::TypeParam) == in_type
        });
        match local {
            Some(l) => Some(Def::Local(file, l)),
            None => self.global(file, name),
        }
    }

    /// An item of `file`, one it imports by name, one of the prelude, or a
    /// module it imports.
    pub fn global(&self, file: usize, name: &str) -> Option<Def> {
        if let Some(at) = self.top_item(file, name) {
            return Some(Def::Item(file, at));
        }
        let imports = &self.files[file].module.imports;
        let targets = || imports.iter().zip(&self.files[file].imports);
        for (import, target) in targets() {
            if let Some(&target) = target
                .as_ref()
                .filter(|_| import.symbols.iter().any(|s| s == name))
                && let Some(at) = self.top_item(target, name)
            {
                return Some(Def::Item(target, at));
            }
        }
        for shared in self.prelude.into_iter().chain([BUILTINS]) {
            if let Some(at) = self.top_item(shared, name)
                && self.files[shared].module.items[at].is_pub
            {
                return Some(Def::Item(shared, at));
            }
        }
        let module =
            targets().find(|(import, _)| import.path.last().is_some_and(|last| last == name));
        module.and_then(|(_, target)| *target).map(Def::Module)
    }

    /// The struct or enum a written type names, seen through references,
    /// pointers and parentheses.
    fn named_type(&self, file: usize, ty: &str) -> Option<(usize, usize)> {
        if let Some((name, _)) = builtin(ty) {
            return Some((BUILTINS, self.top_item(BUILTINS, name)?));
        }
        let (base, _) = generic(strip(ty));
        let def = match base.split_once("::") {
            Some((module, name)) => match self.global(file, module)? {
                Def::Module(target) => Def::Item(target, self.top_item(target, name)?),
                _ => return None,
            },
            None => self.global(file, base)?,
        };
        match def {
            Def::Item(f, at) => Some((f, at)),
            _ => None,
        }
    }

    /// What the use `at` of `file` refers to.
    pub fn resolve(&self, file: usize, at: usize) -> Option<Def> {
        self.resolve_in(file, at, 0)
    }

    fn resolve_in(&self, file: usize, at: usize, depth: usize) -> Option<Def> {
        let module = &self.files[file].module;
        let used = &module.uses[at];
        let name = self.text(file, used.span);
        match &used.kind {
            UseKind::Name => self.lookup(file, name, used.span.start, false),
            UseKind::Type => self.lookup(file, name, used.span.start, true),
            UseKind::Member { object, .. } => {
                let called = self.files[file].text[used.span.end..]
                    .trim_start()
                    .starts_with('(');
                match self.type_of_in(file, object, depth + 1)? {
                    Ty::Value(f, ty) => {
                        let (f, owner) = self.named_type(f, &ty)?;
                        self.member(f, owner, name, called)
                    }
                    Ty::Static(f, owner) => self.member(f, owner, name, called),
                }
            }
            UseKind::Path { qualifier, .. } => {
                match self.resolve_in(file, *qualifier, depth + 1)? {
                    Def::Module(target) => Some(Def::Item(target, self.top_item(target, name)?)),
                    Def::Item(f, owner) => self.child(f, owner, name),
                    Def::Local(..) => None,
                }
            }
            UseKind::Field { owner } => match self.resolve_in(file, *owner, depth + 1)? {
                Def::Item(f, owner) => self.child(f, owner, name),
                _ => None,
            },
            UseKind::Module(k) => self.files[file].imports[*k].map(Def::Module),
            UseKind::Symbol(k) => {
                let target = self.files[file].imports[*k]?;
                Some(Def::Item(target, self.top_item(target, name)?))
            }
        }
    }

    /// The type of what `chain` ends in, as far as the source spells it.
    pub fn type_of(&self, file: usize, chain: &Chain) -> Option<Ty> {
        self.type_of_in(file, chain, 0)
    }

    fn type_of_in(&self, file: usize, chain: &Chain, depth: usize) -> Option<Ty> {
        if depth > 32 {
            return None;
        }
        let mut ops = chain.ops.as_slice();
        let Some(at) = chain.at else {
            return self.apply(Ty::Value(file, chain.ty.clone()?), ops);
        };
        if let Some(number) = self.limit(file, at, depth) {
            return self.apply(Ty::Value(file, number.to_string()), ops);
        }
        let called = ops.first() == Some(&Op::Call);
        let receiver = match &self.files[file].module.uses[at].kind {
            UseKind::Member { object, .. } => self.type_of_in(file, object, depth + 1),
            _ => None,
        };
        let ty = match self.resolve_in(file, at, depth + 1) {
            Some(Def::Local(f, l)) => self.local_type(f, l, depth + 1)?,
            Some(Def::Item(f, i)) => {
                let item = &self.files[f].module.items[i];
                match item.kind {
                    ItemKind::Struct | ItemKind::Enum if ops.first() == Some(&Op::Literal) => {
                        ops = &ops[1..];
                        Ty::Value(f, item.name.clone())
                    }
                    ItemKind::Struct | ItemKind::Enum => Ty::Static(f, i),
                    ItemKind::Const => Ty::Value(f, item.ty.clone()?),
                    ItemKind::Field => self.on_receiver(f, i, item.ty.as_ref()?, receiver),
                    ItemKind::Fn | ItemKind::Method if called => {
                        ops = &ops[1..];
                        self.on_receiver(f, i, item.ty.as_ref()?, receiver)
                    }
                    _ => return None,
                }
            }
            Some(Def::Module(_)) | None => return None,
        };
        self.apply(ty, ops)
    }

    /// The number type `i32.MAX` or `f64.MIN` names, when the use `at` is one.
    fn limit(&self, file: usize, at: usize, depth: usize) -> Option<&str> {
        let uses = &self.files[file].module.uses;
        let UseKind::Member { object, .. } = &uses[at].kind else {
            return None;
        };
        let base = object.at.filter(|_| object.ops.is_empty())?;
        let number = self.text(file, uses[base].span);
        let named = matches!(self.text(file, uses[at].span), "MAX" | "MIN");
        let primitive = PRIMITIVES.contains(&number) && !matches!(number, "bool" | "void");
        (named && primitive && self.resolve_in(file, base, depth + 1).is_none()).then_some(number)
    }

    /// `ty` after indexing, unwrapping, taking references and dereferencing.
    fn apply(&self, mut ty: Ty, ops: &[Op]) -> Option<Ty> {
        for op in ops {
            let Ty::Value(f, written) = ty else {
                return None;
            };
            let written = match op {
                Op::Index => element(&written)?,
                Op::Part(at) => part(&written, *at)?,
                Op::Slice => match strip(&written) {
                    "str" | "String" => "str".to_string(),
                    _ => format!("&[{}]", element(&written)?),
                },
                Op::Unwrap => strip(&written).strip_suffix(['?', '!'])?.to_string(),
                Op::Ref => format!("&{written}"),
                Op::RefVar => format!("&var {written}"),
                Op::Deref => {
                    let inner = written.trim().strip_prefix("&var ");
                    inner
                        .or(written.trim().strip_prefix(['&', '*']))?
                        .to_string()
                }
                _ => return None,
            };
            ty = Ty::Value(f, written);
        }
        Some(ty)
    }

    /// The type a local is given by the source: written, initialized from,
    /// walked by a `for`, or taken out of a payload by a pattern.
    pub fn local_type(&self, file: usize, local: usize, depth: usize) -> Option<Ty> {
        let hint = self.files[file].module.locals[local].hint.as_ref()?;
        self.hint_type(file, hint, depth)
    }

    fn hint_type(&self, file: usize, hint: &Hint, depth: usize) -> Option<Ty> {
        match hint {
            Hint::Written(ty) => Some(Ty::Value(file, ty.clone())),
            Hint::Value(chain) => self.type_of_in(file, chain, depth),
            Hint::Element(chain) => match self.type_of_in(file, chain, depth)? {
                Ty::Value(f, ty) => Some(Ty::Value(f, element(&ty)?)),
                _ => None,
            },
            Hint::Part(whole, at) => match self.hint_type(file, whole, depth)? {
                Ty::Value(f, ty) => Some(Ty::Value(f, part(&ty, *at)?)),
                _ => None,
            },
            Hint::Payload {
                variant,
                index,
                subject,
            } => {
                if let Some(Def::Item(f, v)) = self.resolve_in(file, *variant, depth) {
                    let payload = self.files[f].module.items[v].params.get(*index)?;
                    return Some(Ty::Value(f, payload.clone()));
                }
                let name = self.text(file, self.files[file].module.uses[*variant].span);
                let Ty::Value(f, ty) = self.type_of_in(file, subject, depth)? else {
                    return None;
                };
                let inner = match name {
                    "Some" => strip(&ty).strip_suffix('?')?,
                    "Ok" => strip(&ty).strip_suffix('!')?,
                    "Err" => "i32",
                    _ => return None,
                };
                Some(Ty::Value(f, inner.to_string()))
            }
        }
    }

    /// A member's written type `ty`, with the type parameters of its struct
    /// replaced by the arguments the receiver's written type gives them.
    fn on_receiver(&self, file: usize, member: usize, ty: &str, receiver: Option<Ty>) -> Ty {
        let items = &self.files[file].module.items;
        let params = items[member]
            .owner
            .map_or(&[][..], |owner| &items[owner].type_params);
        if let Some(Ty::Value(f, receiver)) = receiver {
            let args = builtin(&receiver).map_or_else(|| generic(strip(&receiver)).1, |b| b.1);
            let substituted = substitute(ty, params, &args);
            if substituted != ty {
                return Ty::Value(f, substituted);
            }
        }
        Ty::Value(file, ty.to_string())
    }

    /// The struct a value of type `ty`, written in `file`, is.
    pub fn struct_of(&self, file: usize, ty: &str) -> Option<(usize, usize)> {
        let (f, at) = self.named_type(file, ty)?;
        (self.files[f].module.items[at].kind == ItemKind::Struct).then_some((f, at))
    }

    /// The name at `offset` in `file`, declared or used, and what it means.
    pub fn at(&self, file: usize, offset: usize) -> Option<(Span, Def)> {
        let module = &self.files[file].module;
        let items = module.items.iter().enumerate();
        let item = items.map(|(at, item)| (item.name_span, Def::Item(file, at)));
        let locals = module.locals.iter().enumerate();
        let local = locals.map(|(at, local)| (local.span, Def::Local(file, at)));
        if let Some(found) = item.chain(local).find(|(span, _)| span.touches(offset)) {
            return Some(found);
        }
        let used = (0..module.uses.len()).find(|&at| {
            let span = module.uses[at].span;
            span.start < span.end && span.touches(offset)
        })?;
        Some((module.uses[used].span, self.resolve(file, used)?))
    }

    /// What the name at `offset` means, or for a member of a value whose
    /// type is unknown, every field and method so named of a struct the
    /// program declares.
    pub fn meanings(&self, file: usize, offset: usize) -> Option<(Span, Vec<Def>)> {
        if let Some((span, def)) = self.at(file, offset) {
            return Some((span, vec![def]));
        }
        let used = self.files[file].module.uses.iter().find(|used| {
            matches!(used.kind, UseKind::Member { .. }) && used.span.touches(offset)
        })?;
        let name = self.text(file, used.span);
        let mut found = Vec::new();
        for (f, source) in self.files.iter().enumerate().skip(BUILTINS + 1) {
            let items = &source.module.items;
            for (at, item) in items.iter().enumerate() {
                let member = matches!(item.kind, ItemKind::Field | ItemKind::Method);
                let of_struct = item
                    .owner
                    .is_some_and(|o| items[o].kind == ItemKind::Struct);
                if member && of_struct && item.name == name {
                    found.push(Def::Item(f, at));
                }
            }
        }
        (!found.is_empty()).then_some((used.span, found))
    }

    /// Where `def` is declared.
    pub fn declared(&self, def: Def) -> (usize, Span) {
        match def {
            Def::Local(f, at) => (f, self.files[f].module.locals[at].span),
            Def::Item(f, at) => (f, self.files[f].module.items[at].name_span),
            Def::Module(f) => (f, Span::default()),
        }
    }

    /// Every name in the program that refers to `def`, its declaration first.
    pub fn occurrences(&self, def: Def) -> Vec<(usize, Span)> {
        let mut found = match def {
            Def::Module(_) => Vec::new(),
            _ => vec![self.declared(def)],
        };
        for (f, file) in self.files.iter().enumerate() {
            for (at, used) in file.module.uses.iter().enumerate() {
                if used.span.start < used.span.end && self.resolve(f, at) == Some(def) {
                    found.push((f, used.span));
                }
            }
        }
        found
    }

    /// `def` as hover and completion document it: its owner and declaration
    /// in a code block, then its doc comment.
    pub fn markdown(&self, def: Def) -> String {
        let mut code = self.describe(def);
        let mut docs = "";
        if let Def::Item(f, at) = def {
            let items = &self.files[f].module.items;
            if let (Some(owner), ItemKind::Field | ItemKind::Method) =
                (items[at].owner, items[at].kind)
            {
                code = format!("{}\n{code}", items[owner].name);
            }
            docs = &items[at].docs;
        }
        match docs.is_empty() {
            true => format!("```zeru\n{code}\n```"),
            false => format!("```zeru\n{code}\n```\n\n---\n\n{docs}"),
        }
    }

    /// The declaration `def` names, as hover shows it.
    pub fn describe(&self, def: Def) -> String {
        match def {
            Def::Local(f, at) => {
                let local = &self.files[f].module.locals[at];
                match self.local_type(f, at, 0) {
                    Some(Ty::Value(_, ty)) if !local.decl.contains(':') => {
                        format!("{}: {ty}", local.decl)
                    }
                    _ => local.decl.clone(),
                }
            }
            Def::Item(f, at) => {
                let items = &self.files[f].module.items;
                match (items[at].kind, items[at].owner) {
                    (ItemKind::Variant, Some(owner)) => {
                        format!("{}::{}", items[owner].name, items[at].decl)
                    }
                    _ => items[at].decl.clone(),
                }
            }
            Def::Module(f) => format!("module {}", self.files[f].path.display()),
        }
    }
}

/// Every module under `dir`, loaded so completion can offer to import them.
fn modules(dir: &Path) -> Vec<PathBuf> {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    entries
        .flat_map(|entry| match entry.path() {
            path if path.is_dir() => modules(&path),
            path if path.extension().is_some_and(|ext| ext == "zr") => vec![path],
            _ => Vec::new(),
        })
        .collect()
}

/// A type seen through references, pointers and parentheses.
pub fn strip(mut ty: &str) -> &str {
    loop {
        ty = ty.trim();
        if let Some(rest) = ty
            .strip_prefix("&var ")
            .or(ty.strip_prefix('&'))
            .or(ty.strip_prefix('*'))
        {
            ty = rest;
        } else if ty.starts_with('(') && ty.ends_with(')') && !ty.contains(',') {
            ty = &ty[1..ty.len() - 1];
        } else {
            return ty;
        }
    }
}

/// A type's name and its generic arguments: `Vec<Array<i32, 2>>` is `Vec`
/// and `[Array<i32, 2>]`.
pub fn generic(ty: &str) -> (&str, Vec<&str>) {
    let Some((base, rest)) = ty.split_once('<') else {
        return (ty, Vec::new());
    };
    let inner = rest.rsplit_once('>').map_or(rest, |(inner, _)| inner);
    let mut args = Vec::new();
    let (mut depth, mut start) = (0, 0);
    for (at, c) in inner.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(inner[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim());
    (base.trim(), args)
}

/// Part `at` of a tuple type, as `str` is part 1 of `(i32, str)`.
pub fn part(ty: &str, at: usize) -> Option<String> {
    let inner = strip(ty).strip_prefix('(')?.strip_suffix(')')?;
    let listed = format!("_<{inner}>");
    generic(&listed).1.get(at).map(|part| part.to_string())
}

/// The type of an element of a `Vec`, an array, a slice, a `str`, or what
/// `for` gives over a `HashMap` or a `HashSet`.
fn element(ty: &str) -> Option<String> {
    let ty = strip(ty);
    if let Some(inner) = ty.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return Some(inner.to_string());
    }
    match generic(ty) {
        ("Vec" | "Array", args) => args.first().map(|arg| arg.to_string()),
        ("str" | "String", _) => Some("u8".into()),
        ("HashMap", args) => Some(format!("({})", args.join(", "))),
        ("HashSet", args) => args.first().map(|arg| arg.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_split_into_name_and_arguments() {
        assert_eq!(strip("(&var Account)"), "Account");
        assert_eq!(
            generic("HashMap<K, Vec<(i32, i64)>>"),
            ("HashMap", vec!["K", "Vec<(i32, i64)>"])
        );
        assert_eq!(element("&Vec<Account>").as_deref(), Some("Account"));
        assert_eq!(element("&[u8]").as_deref(), Some("u8"));
        assert_eq!(
            part("((i32, bool), Vec<str>)", 0).as_deref(),
            Some("(i32, bool)")
        );
        assert_eq!(part("&(i32, str)", 1).as_deref(), Some("str"));
    }

    #[test]
    fn names_resolve_across_modules() {
        let root = PathBuf::from("/p/main.zr");
        let read = |path: &Path| match path.to_str()? {
            "/p/main.zr" => {
                Some("import geo;\nfn main() { var p = geo::Point { x: 1 }; p.x; p.len(); }".into())
            }
            "/p/geo.zr" => {
                Some("pub struct Point { x: i32, fn len(self) i32 { return self.x; } }".into())
            }
            _ => None,
        };
        let program = Program::load(std::slice::from_ref(&root), None, read);
        let main = program.find(&root).unwrap();
        let geo = program.find(Path::new("/p/geo.zr")).unwrap();
        let text = &program.files[main].text;
        let at = |needle: &str| text.rfind(needle).unwrap();
        assert_eq!(
            program.at(main, at("x;")).map(|(_, d)| d),
            Some(Def::Item(geo, 1))
        );
        assert_eq!(
            program.at(main, at("len")).map(|(_, d)| d),
            Some(Def::Item(geo, 2))
        );
        assert_eq!(
            program.at(main, at("geo::")).map(|(_, d)| d),
            Some(Def::Module(geo))
        );
        assert_eq!(program.occurrences(Def::Item(geo, 1)).len(), 4);
        assert_eq!(program.describe(Def::Local(main, 0)), "var p: Point");
    }

    const TYPED: &str = "enum Shape { Circle(f64), Rect(f64, f64) }
struct Point { x: i32, fn len(self) i32 { return self.x; } }
struct Map<K, V> { keys: Vec<K>, fn get(self, key: K) (&V)? { return None; } }
struct Other { fn len(self) i32 { return 0; } }
fn find(points: &var Vec<Point>) Point? { return None; }
fn main() {
    var points: Vec<Point> = Vec.new();
    var a = &var points[0];
    var b = *a;
    var c = points.get(0).unwrap();
    var d = points.pop()?;
    var e = find(&var points)!;
    for p in &points { p.x; }
    for i in 0..3 { }
    for k, item in points { }
    var m: Map<i64, Point> = make();
    var f = m.get(1);
    var g = m.get(1).copy();
    var s = Shape::Rect(1.0, 2.0);
    match s { Shape::Rect(w, h) => {} default => {} }
    match find(&var points) { Some(q) => {} None => {} }
    var r: i32! = Ok(1);
    match r { Ok(v) => {} Err(code) => {} }
    var t: str = \"a\";
    var byte = t[0];
    var n = 0;
    var x = 1.5 * 2;
    var wide = points.len() as i64;
    var ok = n < 3 && true;
    var neg = -x;
    var txt = \"hi\";
    var ch = 'c';
    var sz = 2 * points.len();
    var part = txt[1..];
    var few = points[..1];
    var lit = Vec[1, 2];
    var table: HashMap<str, Point> = make();
    for (label, val) in table { }
    var u = unknown();
    u.len();
    u.x;
    a.x;
    var sl = points[..1];
    sl.len();
    var front = &var points[..1];
    front.sort();
}
";

    #[test]
    fn types_follow_what_the_source_writes() {
        let root = PathBuf::from("/p/main.zr");
        let read = |_: &Path| Some(TYPED.to_string());
        let program = Program::load(std::slice::from_ref(&root), None, read);
        let main = program.find(&root).unwrap();
        let locals = &program.files[main].module.locals;
        let shown = |name: &str| {
            let at = locals.iter().position(|l| l.name == name).unwrap();
            program.describe(Def::Local(main, at))
        };
        for (name, expected) in [
            ("a", "var a: &var Point"),
            ("b", "var b: Point"),
            ("c", "var c: (&Point)"),
            ("d", "var d: Point"),
            ("e", "var e: Point"),
            ("p", "p: Point"),
            ("i", "i: i32"),
            ("k", "k: usize"),
            ("item", "item: Point"),
            ("f", "var f: (&Point)?"),
            ("g", "var g: Point?"),
            ("h", "h: f64"),
            ("q", "q: Point"),
            ("v", "v: i32"),
            ("code", "code: i32"),
            ("byte", "var byte: u8"),
            ("n", "var n: i32"),
            ("x", "var x: f64"),
            ("wide", "var wide: i64"),
            ("ok", "var ok: bool"),
            ("neg", "var neg: f64"),
            ("txt", "var txt: str"),
            ("ch", "var ch: i32"),
            ("sz", "var sz: usize"),
            ("part", "var part: str"),
            ("few", "var few: &[Point]"),
            ("lit", "var lit: Vec<i32>"),
            ("label", "label: str"),
            ("val", "val: Point"),
        ] {
            assert_eq!(shown(name), expected);
        }
        let at = |needle: &str| TYPED.rfind(needle).unwrap() + 2;
        let meanings = |needle| {
            program
                .meanings(main, at(needle))
                .map(|(_, defs)| defs.len())
        };
        assert_eq!(meanings("u.len"), Some(2));
        assert_eq!(meanings("u.x"), Some(1));
        let labels = |needle| {
            let found = crate::complete::completion(&program, main, at(needle), true);
            found.into_iter().map(|item| item.label).collect::<Vec<_>>()
        };
        assert_eq!(labels("a.x"), ["x", "len"]);
        assert!(!labels("l.len").contains(&"sort".to_string()));
        assert!(labels("t.sort").contains(&"sort".to_string()));
        assert!(labels("u.x").is_empty());
    }
}
