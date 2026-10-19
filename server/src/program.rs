//! The program: the open files, every module they import and the prelude,
//! each parsed on its own, and what each name in them refers to. Types are
//! only the ones written in the source.

use std::path::{Path, PathBuf};

use crate::lexer::Span;
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

/// What a value is known to be: a type written in a file, a struct or enum
/// named as such, or `Vec` named as a type.
#[derive(Debug)]
pub enum Ty {
    Value(usize, String),
    Static(usize, usize),
    Vec,
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
        let mut program = Program {
            files: Vec::new(),
            prelude: None,
        };
        let mut queue = Vec::new();
        if let Some(std) = std {
            let path = std.join("builtin.zr");
            program.prelude = program.add(path, &read, true);
            queue.extend(program.prelude.map(|at| (at, std.to_path_buf())));
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
        if let Some(prelude) = self.prelude
            && let Some(at) = self.top_item(prelude, name)
            && self.files[prelude].module.items[at].is_pub
        {
            return Some(Def::Item(prelude, at));
        }
        let module =
            targets().find(|(import, _)| import.path.last().is_some_and(|last| last == name));
        module.and_then(|(_, target)| *target).map(Def::Module)
    }

    /// The struct or enum a written type names, seen through references,
    /// pointers and parentheses.
    fn named_type(&self, file: usize, ty: &str) -> Option<(usize, usize)> {
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
            UseKind::Member { object, .. } => match self.type_of_in(file, object, depth + 1)? {
                Ty::Value(f, ty) => {
                    let (f, owner) = self.named_type(f, &ty)?;
                    self.child(f, owner, name)
                }
                Ty::Static(f, owner) => self.child(f, owner, name),
                Ty::Vec => None,
            },
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
        let at = chain.at?;
        let mut ops = chain.ops.as_slice();
        let mut ty = match self.resolve_in(file, at, depth + 1) {
            Some(Def::Local(f, l)) => match self.files[f].module.locals[l].hint.as_ref()? {
                Hint::Written(ty) => Ty::Value(f, ty.clone()),
                Hint::Value(chain) => self.type_of_in(f, chain, depth + 1)?,
            },
            Some(Def::Item(f, i)) => {
                let item = &self.files[f].module.items[i];
                match item.kind {
                    ItemKind::Struct | ItemKind::Enum if ops.first() == Some(&Op::Literal) => {
                        ops = &ops[1..];
                        Ty::Value(f, item.name.clone())
                    }
                    ItemKind::Struct | ItemKind::Enum => Ty::Static(f, i),
                    ItemKind::Field | ItemKind::Const => Ty::Value(f, item.ty.clone()?),
                    ItemKind::Fn | ItemKind::Method if ops.first() == Some(&Op::Call) => {
                        ops = &ops[1..];
                        Ty::Value(f, item.ty.clone()?)
                    }
                    _ => return None,
                }
            }
            Some(Def::Module(_)) => return None,
            None => match self.text(file, self.files[file].module.uses[at].span) {
                "Vec" => Ty::Vec,
                _ => return None,
            },
        };
        for op in ops {
            ty = match (op, ty) {
                (Op::Index, Ty::Value(f, ty)) => Ty::Value(f, element(&ty)?),
                (Op::Unwrap, Ty::Value(f, ty)) => {
                    Ty::Value(f, ty.strip_suffix(['?', '!'])?.to_string())
                }
                _ => return None,
            };
        }
        Some(ty)
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

    /// The declaration `def` names, as hover shows it.
    pub fn describe(&self, def: Def) -> String {
        match def {
            Def::Local(f, at) => {
                let local = &self.files[f].module.locals[at];
                let ty = match &local.hint {
                    Some(Hint::Value(chain)) => match self.type_of(f, chain) {
                        Some(Ty::Value(_, ty)) => Some(ty),
                        _ => None,
                    },
                    Some(Hint::Written(ty)) => Some(ty.clone()),
                    None => None,
                };
                match ty {
                    Some(ty) if !local.decl.contains(':') => format!("{}: {ty}", local.decl),
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

/// The type of an element of a `Vec`, an array, a slice or a `str`.
fn element(ty: &str) -> Option<String> {
    let ty = strip(ty);
    if let Some(inner) = ty.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return Some(inner.to_string());
    }
    match generic(ty) {
        ("Vec" | "Array", args) => args.first().map(|arg| arg.to_string()),
        ("str", _) => Some("u8".into()),
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
}
