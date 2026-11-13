//! A small Go symbol index built on tree-sitter.
//!
//! It answers two questions for the editor — what can be typed here, and where
//! is this name defined — from syntax alone. There is no type checker: types
//! are followed by name through declarations, which covers locals, package
//! symbols, struct fields, methods, and members of imported packages.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use tree_sitter::{Node, Parser, Tree};

const MAX_ITEMS: usize = 60;
/// How many declarations a type may be followed through before giving up.
const MAX_DEPTH: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Local,
    Func,
    Method,
    Type,
    Var,
    Const,
    Field,
    Package,
    Builtin,
    Keyword,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: Kind,
    /// Signature or type, shown next to the name.
    pub detail: String,
    /// The type a method or field belongs to.
    pub owner: Option<String>,
    /// Type expression of a variable or field; first result of a function.
    pub ty: Option<String>,
    /// An embedded field: its members are also members of the owner.
    pub embedded: bool,
    pub file: Arc<Path>,
    pub row: u32,
    /// Byte column of the name within its line.
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Import {
    /// The name the package is referred to by in this file.
    pub name: String,
    pub path: String,
}

#[derive(Default)]
pub struct Package {
    pub symbols: Vec<Symbol>,
    imports: HashMap<Arc<Path>, Vec<Import>>,
}

/// A type expression together with the file whose imports give it meaning.
#[derive(Clone, Debug)]
struct Ty {
    text: String,
    file: Arc<Path>,
}

struct Module {
    root: PathBuf,
    path: String,
    requires: Vec<(String, String)>,
    /// `replace old => target [version]`; a target starting with `.` is a path.
    replaces: Vec<(String, String, Option<String>)>,
}

type Stamp = Vec<(PathBuf, SystemTime)>;

/// Parsed packages, cached by directory.
#[derive(Default)]
pub struct Index {
    packages: Mutex<HashMap<PathBuf, (Option<Stamp>, Arc<Package>)>>,
    /// Symbols declared in each directory's `_test.go` files.
    tests: Mutex<HashMap<PathBuf, (Option<Stamp>, Arc<Package>)>>,
    modules: Mutex<HashMap<PathBuf, (Option<SystemTime>, Arc<Module>)>>,
}

pub struct Item {
    pub name: String,
    pub kind: Kind,
    pub detail: String,
}

#[derive(Default)]
pub struct Completions {
    /// Byte offset where the word being completed starts.
    pub start: usize,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub file: PathBuf,
    pub row: u32,
    pub col: u32,
}

// ---------------------------------------------------------------- parsing

fn parse(source: &str) -> Option<Tree> {
    thread_local! {
        static PARSER: RefCell<Parser> = RefCell::new({
            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_go::LANGUAGE.into())
                .expect("the Go grammar matches this tree-sitter");
            parser
        });
    }
    PARSER.with(|parser| parser.borrow_mut().parse(source, None))
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or_default()
}

/// Source text on one line, for display.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.len() > 80 {
        let mut end = 77;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push('…');
    }
    out
}

fn signature(node: Node, src: &str) -> String {
    let mut out = String::from("func");
    if let Some(parameters) = node.child_by_field_name("parameters") {
        out.push_str(text(parameters, src));
    }
    if let Some(result) = node.child_by_field_name("result") {
        out.push(' ');
        out.push_str(text(result, src));
    }
    one_line(&out)
}

/// The type of a function's first result, as written.
fn first_result(node: Node, src: &str) -> Option<String> {
    let result = node.child_by_field_name("result")?;
    if result.kind() != "parameter_list" {
        return Some(text(result, src).to_string());
    }
    let mut cursor = result.walk();
    let first = result.named_children(&mut cursor).next()?;
    let ty = first.child_by_field_name("type")?;
    Some(text(ty, src).to_string())
}

fn symbol(name: Node, src: &str, kind: Kind, detail: String, file: &Arc<Path>) -> Symbol {
    let position = name.start_position();
    Symbol {
        name: text(name, src).to_string(),
        kind,
        detail,
        owner: None,
        ty: None,
        embedded: false,
        file: file.clone(),
        row: position.row as u32,
        col: position.column as u32,
    }
}

/// The last path segment a type expression names: `*pkg.T[K]` gives `T`.
fn base_name(ty: &str) -> &str {
    let ty = ty.trim_start_matches(['*', '&']).trim();
    let ty = ty.split('[').next().unwrap_or(ty);
    ty.rsplit('.').next().unwrap_or(ty).trim()
}

fn file_symbols(src: &str, tree: &Tree, file: &Arc<Path>) -> (Vec<Import>, Vec<Symbol>) {
    let mut imports = Vec::new();
    let mut symbols = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "import_declaration" => collect_imports(node, src, &mut imports),
            "function_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let mut sym = symbol(name, src, Kind::Func, signature(node, src), file);
                    sym.ty = first_result(node, src);
                    symbols.push(sym);
                }
            }
            "method_declaration" => {
                let owner = node
                    .child_by_field_name("receiver")
                    .and_then(|receiver| {
                        let mut cursor = receiver.walk();
                        receiver.named_children(&mut cursor).next()
                    })
                    .and_then(|parameter| parameter.child_by_field_name("type"))
                    .map(|ty| base_name(text(ty, src)).to_string());
                if let (Some(name), Some(owner)) = (node.child_by_field_name("name"), owner) {
                    let mut sym = symbol(name, src, Kind::Method, signature(node, src), file);
                    sym.ty = first_result(node, src);
                    sym.owner = Some(owner);
                    symbols.push(sym);
                }
            }
            "type_declaration" => {
                let mut cursor = node.walk();
                for spec in node.named_children(&mut cursor) {
                    collect_type(spec, src, file, &mut symbols);
                }
            }
            "var_declaration" | "const_declaration" => {
                let kind = if node.kind() == "var_declaration" {
                    Kind::Var
                } else {
                    Kind::Const
                };
                for_each_spec(node, &mut |spec| {
                    let ty = spec
                        .child_by_field_name("type")
                        .map(|ty| text(ty, src).to_string());
                    let detail = ty.clone().map(|ty| one_line(&ty)).unwrap_or_else(|| {
                        spec.child_by_field_name("value")
                            .map(|value| one_line(&format!("= {}", text(value, src))))
                            .unwrap_or_default()
                    });
                    let mut cursor = spec.walk();
                    for name in spec.children_by_field_name("name", &mut cursor) {
                        if name.kind() != "identifier" {
                            continue;
                        }
                        let mut sym = symbol(name, src, kind, detail.clone(), file);
                        sym.ty = ty.clone();
                        symbols.push(sym);
                    }
                });
            }
            _ => {}
        }
    }
    (imports, symbols)
}

/// Visit every `var_spec` / `const_spec` of a declaration, grouped or not.
fn for_each_spec<'a>(node: Node<'a>, visit: &mut dyn FnMut(Node<'a>)) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "var_spec" | "const_spec" => visit(child),
            "var_spec_list" => for_each_spec(child, visit),
            _ => {}
        }
    }
}

fn collect_imports(node: Node, src: &str, imports: &mut Vec<Import>) {
    if node.kind() == "import_spec" {
        let Some(path) = node.child_by_field_name("path") else {
            return;
        };
        let path = text(path, src).trim_matches(['"', '`']).to_string();
        let name = match node.child_by_field_name("name") {
            Some(name) => text(name, src).to_string(),
            None => default_package_name(&path).to_string(),
        };
        imports.push(Import { name, path });
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_imports(child, src, imports);
    }
}

/// The package name an import path most likely declares.
fn default_package_name(path: &str) -> &str {
    let mut segments = path.rsplit('/');
    let mut last = segments.next().unwrap_or(path);
    // `example.com/mod/v2` names the package of the segment before the version.
    if last.len() > 1 && last.starts_with('v') && last[1..].bytes().all(|b| b.is_ascii_digit()) {
        last = segments.next().unwrap_or(last);
    }
    // `gopkg.in/yaml.v3` declares package `yaml`.
    if let Some((name, version)) = last.rsplit_once(".v")
        && !version.is_empty()
        && version.bytes().all(|b| b.is_ascii_digit())
    {
        last = name;
    }
    last.strip_prefix("go-").unwrap_or(last)
}

fn collect_type(spec: Node, src: &str, file: &Arc<Path>, symbols: &mut Vec<Symbol>) {
    let (Some(name), Some(ty)) = (
        spec.child_by_field_name("name"),
        spec.child_by_field_name("type"),
    ) else {
        return;
    };
    let owner = text(name, src).to_string();
    let detail = match ty.kind() {
        "struct_type" => "struct".to_string(),
        "interface_type" => "interface".to_string(),
        _ => one_line(text(ty, src)),
    };
    let mut sym = symbol(name, src, Kind::Type, detail, file);
    match ty.kind() {
        "struct_type" => {
            let mut stack = vec![ty];
            while let Some(node) = stack.pop() {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "field_declaration" {
                        collect_field(child, src, file, &owner, symbols);
                    } else if child.kind() == "field_declaration_list" {
                        stack.push(child);
                    }
                }
            }
        }
        "interface_type" => {
            let mut cursor = ty.walk();
            for child in ty.named_children(&mut cursor) {
                match child.kind() {
                    "method_elem" | "method_spec" => {
                        if let Some(name) = child.child_by_field_name("name") {
                            let mut method =
                                symbol(name, src, Kind::Method, signature(child, src), file);
                            method.ty = first_result(child, src);
                            method.owner = Some(owner.clone());
                            symbols.push(method);
                        }
                    }
                    "type_elem" => {
                        let embedded = text(child, src);
                        let mut field = symbol(child, src, Kind::Field, one_line(embedded), file);
                        field.name = base_name(embedded).to_string();
                        field.ty = Some(embedded.to_string());
                        field.owner = Some(owner.clone());
                        field.embedded = true;
                        symbols.push(field);
                    }
                    _ => {}
                }
            }
        }
        // `type A B`: A has B's fields.
        _ => sym.ty = Some(text(ty, src).to_string()),
    }
    symbols.push(sym);
}

fn collect_field(node: Node, src: &str, file: &Arc<Path>, owner: &str, out: &mut Vec<Symbol>) {
    let Some(ty) = node.child_by_field_name("type") else {
        return;
    };
    // The type as written, including a `*` that sits outside the type node.
    let ty_text = {
        let full = text(node, src);
        let start = ty.start_byte() - node.start_byte();
        let end = ty.end_byte() - node.start_byte();
        let star = full[..start].trim_end().ends_with('*');
        format!("{}{}", if star { "*" } else { "" }, &full[start..end])
    };
    let mut cursor = node.walk();
    let names: Vec<Node> = node.children_by_field_name("name", &mut cursor).collect();
    if names.is_empty() {
        let mut field = symbol(ty, src, Kind::Field, one_line(&ty_text), file);
        field.name = base_name(&ty_text).to_string();
        field.ty = Some(ty_text);
        field.owner = Some(owner.to_string());
        field.embedded = true;
        out.push(field);
        return;
    }
    for name in names {
        let mut field = symbol(name, src, Kind::Field, one_line(&ty_text), file);
        field.ty = Some(ty_text.clone());
        field.owner = Some(owner.to_string());
        out.push(field);
    }
}

// ---------------------------------------------------------------- go env

struct GoEnv {
    goroot: Option<PathBuf>,
    modcache: Option<PathBuf>,
}

fn go_env() -> &'static GoEnv {
    static ENV: OnceLock<GoEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let output = crate::proc::run("go", &["env", "GOROOT", "GOMODCACHE"], None);
        let mut lines = output.as_deref().unwrap_or_default().lines();
        let mut next = || {
            lines
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
        };
        GoEnv {
            goroot: next(),
            modcache: next(),
        }
    })
}

/// Files of another operating system are not part of this build.
fn built_here(file_name: &str) -> bool {
    const OTHERS: &[&str] = &[
        "aix", "android", "darwin", "dragonfly", "freebsd", "illumos", "ios", "js", "linux",
        "netbsd", "openbsd", "plan9", "solaris", "wasip1", "windows", "unix",
    ];
    let host = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let stem = file_name.trim_end_matches(".go").trim_end_matches("_test");
    let mut parts = stem.rsplit('_');
    let last = parts.next().unwrap_or_default();
    let before = parts.next().unwrap_or_default();
    let unix_host = host != "windows";
    ![last, before].iter().any(|part| {
        OTHERS.contains(part) && *part != host && !(*part == "unix" && unix_host)
    })
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// The go.mod that governs `dir`.
    fn module(&self, dir: &Path) -> Option<Arc<Module>> {
        let root = dir.ancestors().find(|dir| dir.join("go.mod").is_file())?;
        let go_mod = root.join("go.mod");
        let modified = std::fs::metadata(&go_mod).and_then(|m| m.modified()).ok();
        let mut modules = self.modules.lock().unwrap();
        if let Some((stamp, module)) = modules.get(root)
            && *stamp == modified
        {
            return Some(module.clone());
        }
        let module = Arc::new(parse_go_mod(
            root,
            &std::fs::read_to_string(&go_mod).unwrap_or_default(),
        ));
        modules.insert(root.to_path_buf(), (modified, module.clone()));
        Some(module)
    }

    /// The directory holding the package `import` names, as seen from `from`.
    fn resolve_import(&self, from: &Path, import: &str) -> Option<PathBuf> {
        let env = go_env();
        let exists = |dir: PathBuf| dir.is_dir().then_some(dir);
        let module = self.module(from);
        if let Some(module) = &module {
            if import == module.path {
                return Some(module.root.clone());
            }
            if let Some(rest) = import
                .strip_prefix(module.path.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                && let Some(dir) = exists(module.root.join(rest))
            {
                return Some(dir);
            }
            if let Some(dir) = exists(module.root.join("vendor").join(import)) {
                return Some(dir);
            }
        }
        // Standard library paths have no dot in their first segment.
        if !import.split('/').next().unwrap_or_default().contains('.')
            && let Some(goroot) = &env.goroot
            && let Some(dir) = exists(goroot.join("src").join(import))
        {
            return Some(dir);
        }
        let module = module?;
        let longest = |candidates: &mut dyn Iterator<Item = (&str, usize)>| {
            candidates
                .filter(|(path, _)| {
                    import == *path
                        || import
                            .strip_prefix(path)
                            .is_some_and(|rest| rest.starts_with('/'))
                })
                .max_by_key(|(path, _)| path.len())
                .map(|(path, ix)| (path.len(), ix))
        };
        let in_cache = |module_path: &str, version: &str, rest: &str| {
            let modcache = env.modcache.as_ref()?;
            let mut escaped = String::new();
            for ch in module_path.chars() {
                if ch.is_ascii_uppercase() {
                    escaped.push('!');
                    escaped.push(ch.to_ascii_lowercase());
                } else {
                    escaped.push(ch);
                }
            }
            exists(
                modcache
                    .join(format!("{escaped}@{version}"))
                    .join(rest.trim_start_matches('/')),
            )
        };
        let mut replaces = module
            .replaces
            .iter()
            .enumerate()
            .map(|(ix, (old, _, _))| (old.as_str(), ix));
        if let Some((len, ix)) = longest(&mut replaces) {
            let (_, target, version) = &module.replaces[ix];
            let rest = &import[len..];
            return match version {
                Some(version) => in_cache(target, version, rest),
                None => exists(module.root.join(target).join(rest.trim_start_matches('/'))),
            };
        }
        let mut requires = module
            .requires
            .iter()
            .enumerate()
            .map(|(ix, (path, _))| (path.as_str(), ix));
        let (len, ix) = longest(&mut requires)?;
        let (path, version) = &module.requires[ix];
        in_cache(path, version, &import[len..])
    }

    /// The symbols of the package in `dir`. Test files are left out.
    pub fn package(&self, dir: &Path) -> Arc<Package> {
        self.load(dir, false)
    }

    /// The symbols declared in the `_test.go` files of `dir`.
    fn test_package(&self, dir: &Path) -> Arc<Package> {
        self.load(dir, true)
    }

    fn load(&self, dir: &Path, tests: bool) -> Arc<Package> {
        let cache = if tests { &self.tests } else { &self.packages };
        let env = go_env();
        // The toolchain and the module cache never change under us.
        let immutable = [&env.goroot, &env.modcache]
            .into_iter()
            .flatten()
            .any(|root| dir.starts_with(root));
        if immutable && let Some((_, package)) = cache.lock().unwrap().get(dir) {
            return package.clone();
        }

        let mut files: Stamp = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".go") || name.ends_with("_test.go") != tests || !built_here(&name) {
                    return None;
                }
                let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
                Some((entry.path(), modified))
            })
            .collect();
        files.sort();

        if let Some((Some(stamp), package)) = cache.lock().unwrap().get(dir)
            && *stamp == files
        {
            return package.clone();
        }

        let mut package = Package::default();
        let mut seen = HashSet::new();
        for (path, _) in &files {
            let Ok(src) = std::fs::read_to_string(path) else {
                continue;
            };
            let Some(tree) = parse(&src) else {
                continue;
            };
            let file: Arc<Path> = Arc::from(path.as_path());
            let (imports, symbols) = file_symbols(&src, &tree, &file);
            package.imports.insert(file, imports);
            for symbol in symbols {
                if seen.insert((symbol.owner.clone(), symbol.name.clone())) {
                    package.symbols.push(symbol);
                }
            }
        }
        let package = Arc::new(package);
        cache.lock().unwrap().insert(
            dir.to_path_buf(),
            ((!immutable).then_some(files), package.clone()),
        );
        package
    }
}

fn parse_go_mod(root: &Path, content: &str) -> Module {
    let mut module = Module {
        root: root.to_path_buf(),
        path: String::new(),
        requires: Vec::new(),
        replaces: Vec::new(),
    };
    let mut block = "";
    for line in content.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            block = "";
            continue;
        }
        let (directive, rest) = if block.is_empty() {
            let (directive, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            (directive, rest.trim())
        } else {
            (block, line)
        };
        if rest == "(" {
            block = match directive {
                "require" => "require",
                "replace" => "replace",
                _ => "other",
            };
            continue;
        }
        let mut words = rest.split_whitespace();
        match directive {
            "module" => module.path = rest.trim_matches('"').to_string(),
            "require" => {
                if let (Some(path), Some(version)) = (words.next(), words.next()) {
                    module.requires.push((path.to_string(), version.to_string()));
                }
            }
            "replace" => {
                let words: Vec<&str> = words.collect();
                if let Some(arrow) = words.iter().position(|word| *word == "=>")
                    && let (Some(old), Some(target)) = (words.first(), words.get(arrow + 1))
                {
                    let version = words.get(arrow + 2).map(|v| v.to_string());
                    // Without a version the target is a directory.
                    module
                        .replaces
                        .push((old.to_string(), target.to_string(), version));
                }
            }
            _ => {}
        }
    }
    module
}

// ---------------------------------------------------------------- scope

/// How a local name was introduced, which is where its type comes from.
#[derive(Clone, Copy)]
enum Origin<'a> {
    /// Declared with this type.
    Typed(Node<'a>),
    /// The `index`-th of `count` names initialized from this expression list.
    Value {
        values: Node<'a>,
        index: usize,
        count: usize,
    },
    /// The `index`-th name of a `range` over this expression.
    Range { operand: Node<'a>, index: usize },
    Unknown,
}

struct Local<'a> {
    name: &'a str,
    node: Node<'a>,
    origin: Origin<'a>,
}

/// Everything known about one position in one file.
struct Scope<'a> {
    index: &'a Index,
    file: Arc<Path>,
    dir: PathBuf,
    src: &'a str,
    imports: Vec<Import>,
    /// This file's package, with this file as it is in the editor.
    package: Arc<Package>,
    /// Innermost declarations last.
    locals: Vec<Local<'a>>,
}

const SCOPES: &[&str] = &[
    "block",
    "if_statement",
    "for_statement",
    "expression_switch_statement",
    "type_switch_statement",
    "select_statement",
    "expression_case",
    "type_case",
    "default_case",
    "communication_case",
    "func_literal",
    "function_declaration",
    "method_declaration",
];

fn collect_locals<'a>(node: Node<'a>, src: &'a str, offset: usize, out: &mut Vec<Local<'a>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() >= offset {
            break;
        }
        let kind = child.kind();
        // A scope that closed before the cursor hides what it declared.
        if SCOPES.contains(&kind) && child.end_byte() < offset {
            continue;
        }
        match kind {
            "parameter_declaration" | "variadic_parameter_declaration" => {
                let origin = child
                    .child_by_field_name("type")
                    .map_or(Origin::Unknown, Origin::Typed);
                let mut cursor = child.walk();
                for name in child.children_by_field_name("name", &mut cursor) {
                    push_local(name, src, origin, out);
                }
                continue;
            }
            "var_spec" | "const_spec" => {
                let ty = child.child_by_field_name("type");
                let values = child.child_by_field_name("value");
                let mut cursor = child.walk();
                let names: Vec<Node> = child
                    .children_by_field_name("name", &mut cursor)
                    .filter(|name| name.kind() == "identifier")
                    .collect();
                for (index, name) in names.iter().enumerate() {
                    let origin = match (ty, values) {
                        (Some(ty), _) => Origin::Typed(ty),
                        (None, Some(values)) => Origin::Value {
                            values,
                            index,
                            count: names.len(),
                        },
                        _ => Origin::Unknown,
                    };
                    push_local(*name, src, origin, out);
                }
                continue;
            }
            "short_var_declaration" | "range_clause" | "receive_statement" => {
                let (Some(left), Some(right)) = (
                    child.child_by_field_name("left"),
                    child.child_by_field_name("right"),
                ) else {
                    continue;
                };
                // `x = <-ch` assigns; only `:=` declares.
                if kind != "short_var_declaration" && !text(child, src).contains(":=") {
                    continue;
                }
                let mut cursor = left.walk();
                let names: Vec<Node> = left.named_children(&mut cursor).collect();
                for (index, name) in names.iter().enumerate() {
                    if name.kind() != "identifier" {
                        continue;
                    }
                    let origin = if kind == "range_clause" {
                        Origin::Range {
                            operand: right,
                            index,
                        }
                    } else {
                        Origin::Value {
                            values: right,
                            index,
                            count: names.len(),
                        }
                    };
                    push_local(*name, src, origin, out);
                }
                // The right-hand side can hold function literals the cursor is in.
                collect_locals(right, src, offset, out);
                continue;
            }
            "type_switch_statement" => {
                if let Some(alias) = child.child_by_field_name("alias") {
                    let mut cursor = alias.walk();
                    for name in alias.named_children(&mut cursor) {
                        push_local(name, src, Origin::Unknown, out);
                    }
                }
            }
            _ => {}
        }
        collect_locals(child, src, offset, out);
    }
}

fn push_local<'a>(name: Node<'a>, src: &'a str, origin: Origin<'a>, out: &mut Vec<Local<'a>>) {
    let name_text = text(name, src);
    if name.kind() == "identifier" && name_text != "_" {
        out.push(Local {
            name: name_text,
            node: name,
            origin,
        });
    }
}

impl<'a> Scope<'a> {
    fn new(index: &'a Index, path: &Path, src: &'a str, tree: &'a Tree, offset: usize) -> Self {
        let file: Arc<Path> = Arc::from(path);
        let dir = path.parent().map(PathBuf::from).unwrap_or_default();
        let (imports, symbols) = file_symbols(src, tree, &file);

        // The package as it is on disk, with this file replaced by the editor's text.
        let on_disk = index.package(&dir);
        let mut package = Package {
            symbols,
            imports: on_disk.imports.clone(),
        };
        package.imports.insert(file.clone(), imports.clone());
        let mut seen: HashSet<(Option<String>, String)> = package
            .symbols
            .iter()
            .map(|s| (s.owner.clone(), s.name.clone()))
            .collect();
        for symbol in &on_disk.symbols {
            if symbol.file != file && seen.insert((symbol.owner.clone(), symbol.name.clone())) {
                package.symbols.push(symbol.clone());
            }
        }

        let mut locals = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();
        for node in root.children(&mut cursor) {
            if node.start_byte() >= offset {
                break;
            }
            // Top-level declarations are package symbols, not locals; only the
            // function (or broken code) around the cursor declares locals.
            let function = matches!(
                node.kind(),
                "function_declaration" | "method_declaration" | "ERROR"
            );
            if function && node.end_byte() >= offset {
                collect_locals(node, src, offset, &mut locals);
            }
        }

        Self {
            index,
            file,
            dir,
            src,
            imports,
            package: Arc::new(package),
            locals,
        }
    }

    /// A scope with no file of its own: the package in `dir`, test files included.
    fn for_dir(index: &'a Index, dir: &Path) -> Self {
        let base = index.package(dir);
        let tests = index.test_package(dir);
        let mut package = Package {
            symbols: base.symbols.clone(),
            imports: base.imports.clone(),
        };
        package.symbols.extend(tests.symbols.iter().cloned());
        package.imports.extend(
            tests
                .imports
                .iter()
                .map(|(file, imports)| (file.clone(), imports.clone())),
        );
        Self {
            index,
            file: Arc::from(dir.join("_").as_path()),
            dir: dir.to_path_buf(),
            src: "",
            imports: Vec::new(),
            package: Arc::new(package),
            locals: Vec::new(),
        }
    }

    fn package_of(&self, dir: &Path) -> Arc<Package> {
        if dir == self.dir {
            self.package.clone()
        } else {
            self.index.package(dir)
        }
    }

    fn ty(&self, text: impl Into<String>) -> Ty {
        Ty {
            text: text.into(),
            file: self.file.clone(),
        }
    }

    fn local(&self, name: &str) -> Option<&Local<'a>> {
        self.locals.iter().rev().find(|local| local.name == name)
    }

    fn import_dir(&self, file: &Path, name: &str) -> Option<PathBuf> {
        let dir = file.parent()?;
        let package = self.package_of(dir);
        let import = package
            .imports
            .get(file)?
            .iter()
            .find(|import| import.name == name)?;
        self.index.resolve_import(dir, &import.path)
    }

    /// The package directory and name of the named type `ty` refers to.
    fn named(&self, ty: &Ty) -> Option<(PathBuf, String)> {
        let text = ty.text.trim().trim_start_matches(['*', '&']).trim();
        let text = text.split('[').next().unwrap_or(text).trim();
        if text.is_empty() || !text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            return None;
        }
        match text.split_once('.') {
            Some((package, name)) => {
                Some((self.import_dir(&ty.file, package)?, name.to_string()))
            }
            None => Some((ty.file.parent()?.to_path_buf(), text.to_string())),
        }
    }

    /// Fields and methods of `ty`, including those of embedded types.
    fn members(&self, ty: &Ty, depth: u8) -> Vec<Symbol> {
        let Some((dir, name)) = self.named(ty) else {
            return Vec::new();
        };
        if depth == 0 {
            return Vec::new();
        }
        let package = self.package_of(&dir);
        let mut out: Vec<Symbol> = package
            .symbols
            .iter()
            .filter(|s| s.owner.as_deref() == Some(name.as_str()))
            .cloned()
            .collect();
        let mut inherited = Vec::new();
        for field in out.iter().filter(|s| s.embedded) {
            if let Some(text) = &field.ty {
                inherited.push(Ty {
                    text: text.clone(),
                    file: field.file.clone(),
                });
            }
        }
        // `type A B` has B's fields.
        if let Some(underlying) = package
            .symbols
            .iter()
            .find(|s| s.kind == Kind::Type && s.owner.is_none() && s.name == name)
            && let Some(text) = &underlying.ty
        {
            inherited.push(Ty {
                text: text.clone(),
                file: underlying.file.clone(),
            });
        }
        for ty in inherited {
            for member in self.members(&ty, depth - 1) {
                if !out.iter().any(|s| s.name == member.name) {
                    out.push(member);
                }
            }
        }
        out
    }

    fn package_symbol(&self, dir: &Path, name: &str) -> Option<Symbol> {
        self.package_of(dir)
            .symbols
            .iter()
            .find(|s| s.owner.is_none() && s.name == name)
            .cloned()
    }

    /// What `operand.field` names.
    fn member(&self, operand: Node, src: &str, field: &str, depth: u8) -> Option<Symbol> {
        if depth == 0 {
            return None;
        }
        if operand.kind() == "identifier" {
            let name = text(operand, src);
            if self.local(name).is_none()
                && let Some(dir) = self.import_dir(&self.file, name)
            {
                return self.package_symbol(&dir, field);
            }
        }
        let ty = self.expr_ty(operand, src, depth - 1)?;
        self.members(&ty, MAX_DEPTH)
            .into_iter()
            .find(|member| member.name == field)
    }

    fn symbol_ty(&self, symbol: &Symbol) -> Option<Ty> {
        if symbol.kind == Kind::Type {
            return Some(Ty {
                text: symbol.name.clone(),
                file: symbol.file.clone(),
            });
        }
        Some(Ty {
            text: symbol.ty.clone()?,
            file: symbol.file.clone(),
        })
    }

    fn local_ty(&self, local: &Local<'a>, depth: u8) -> Option<Ty> {
        if depth == 0 {
            return None;
        }
        match local.origin {
            Origin::Typed(ty) => Some(self.ty(text(ty, self.src))),
            Origin::Value {
                values,
                index,
                count,
            } => {
                let mut cursor = values.walk();
                let exprs: Vec<Node> = if values.kind() == "expression_list" {
                    values.named_children(&mut cursor).collect()
                } else {
                    vec![values]
                };
                if exprs.len() == count {
                    self.expr_ty(*exprs.get(index)?, self.src, depth - 1)
                } else if index == 0 {
                    // `x, err := f()`: only the first result's type is known.
                    self.expr_ty(*exprs.first()?, self.src, depth - 1)
                } else {
                    None
                }
            }
            Origin::Range { operand, index } => {
                let ty = self.expr_ty(operand, self.src, depth - 1)?;
                let text = ty.text.trim().trim_start_matches('*');
                let element = if let Some(rest) = text.strip_prefix("map[") {
                    let close = matching_bracket(rest)?;
                    if index == 0 {
                        &rest[..close]
                    } else {
                        &rest[close + 1..]
                    }
                } else if index == 1 && text.starts_with('[') {
                    &text[text.find(']')? + 1..]
                } else if index == 0 && text.starts_with("chan ") {
                    &text[5..]
                } else {
                    return None;
                };
                Some(Ty {
                    text: element.trim().to_string(),
                    file: ty.file,
                })
            }
            Origin::Unknown => None,
        }
    }

    /// The type of an expression, as far as declarations tell.
    fn expr_ty(&self, node: Node, src: &str, depth: u8) -> Option<Ty> {
        if depth == 0 {
            return None;
        }
        match node.kind() {
            "identifier" => {
                let name = text(node, src);
                if let Some(local) = self.local(name) {
                    return self.local_ty(local, depth - 1);
                }
                let symbol = self.package_symbol(&self.dir, name)?;
                self.symbol_ty(&symbol)
            }
            "composite_literal" | "type_assertion_expression" | "type_conversion_expression" => {
                Some(self.ty(text(node.child_by_field_name("type")?, src)))
            }
            "unary_expression" => {
                let operand = self.expr_ty(node.child_by_field_name("operand")?, src, depth - 1)?;
                let operator = node.child_by_field_name("operator").map(|op| op.kind());
                match operator {
                    Some("<-") => Some(Ty {
                        text: operand.text.trim().strip_prefix("chan ")?.to_string(),
                        file: operand.file,
                    }),
                    _ => Some(operand),
                }
            }
            "parenthesized_expression" => {
                let mut cursor = node.walk();
                let inner = node.named_children(&mut cursor).next()?;
                self.expr_ty(inner, src, depth - 1)
            }
            "selector_expression" => {
                let operand = node.child_by_field_name("operand")?;
                let field = text(node.child_by_field_name("field")?, src);
                let symbol = self.member(operand, src, field, depth - 1)?;
                self.symbol_ty(&symbol)
            }
            "index_expression" => {
                let operand = self.expr_ty(node.child_by_field_name("operand")?, src, depth - 1)?;
                let text = operand.text.trim().trim_start_matches('*');
                let element = if let Some(rest) = text.strip_prefix("map[") {
                    &rest[matching_bracket(rest)? + 1..]
                } else if text.starts_with('[') {
                    &text[text.find(']')? + 1..]
                } else {
                    return None;
                };
                Some(Ty {
                    text: element.trim().to_string(),
                    file: operand.file,
                })
            }
            "call_expression" => {
                let function = node.child_by_field_name("function")?;
                let first_argument = || {
                    let arguments = node.child_by_field_name("arguments")?;
                    let mut cursor = arguments.walk();
                    arguments.named_children(&mut cursor).next()
                };
                match function.kind() {
                    "identifier" => {
                        let name = text(function, src);
                        match name {
                            "new" | "make" => Some(self.ty(text(first_argument()?, src))),
                            "append" => self.expr_ty(first_argument()?, src, depth - 1),
                            _ if self.local(name).is_some() => None,
                            _ => {
                                let symbol = self.package_symbol(&self.dir, name)?;
                                self.symbol_ty(&symbol)
                            }
                        }
                    }
                    "selector_expression" => {
                        let operand = function.child_by_field_name("operand")?;
                        let field = text(function.child_by_field_name("field")?, src);
                        let symbol = self.member(operand, src, field, depth - 1)?;
                        self.symbol_ty(&symbol)
                    }
                    _ => None,
                }
            }
            "interpreted_string_literal" | "raw_string_literal" => Some(self.ty("string")),
            "int_literal" => Some(self.ty("int")),
            "float_literal" => Some(self.ty("float64")),
            "true" | "false" => Some(self.ty("bool")),
            _ => None,
        }
    }

    /// What can follow `receiver.`.
    fn members_of(&self, receiver: &str) -> Vec<Symbol> {
        // The receiver is parsed on its own: the file around the cursor is
        // usually mid-edit and does not parse as written.
        let snippet = format!("package p\nvar _ = {receiver}\n");
        let Some(tree) = parse(&snippet) else {
            return Vec::new();
        };
        let Some(expr) = find_kind(tree.root_node(), "var_spec")
            .and_then(|spec| spec.child_by_field_name("value"))
            .and_then(|values| {
                let mut cursor = values.walk();
                values.named_children(&mut cursor).next()
            })
        else {
            return Vec::new();
        };

        if expr.kind() == "identifier" {
            let name = text(expr, &snippet);
            if self.local(name).is_none()
                && let Some(dir) = self.import_dir(&self.file, name)
            {
                let foreign = dir != self.dir;
                return self
                    .package_of(&dir)
                    .symbols
                    .iter()
                    .filter(|s| s.owner.is_none() && (!foreign || exported(&s.name)))
                    .cloned()
                    .collect();
            }
        }
        let Some(ty) = self.expr_ty(expr, &snippet, MAX_DEPTH) else {
            return Vec::new();
        };
        let foreign = self
            .named(&ty)
            .is_some_and(|(dir, _)| dir != self.dir);
        self.members(&ty, MAX_DEPTH)
            .into_iter()
            .filter(|s| !foreign || exported(&s.name))
            .collect()
    }

    fn local_symbol(&self, local: &Local<'a>) -> Symbol {
        let position = local.node.start_position();
        let detail = match local.origin {
            Origin::Typed(ty) => one_line(text(ty, self.src)),
            _ => self
                .local_ty(local, MAX_DEPTH)
                .map(|ty| one_line(&ty.text))
                .unwrap_or_default(),
        };
        Symbol {
            name: local.name.to_string(),
            kind: Kind::Local,
            detail,
            owner: None,
            ty: None,
            embedded: false,
            file: self.file.clone(),
            row: position.row as u32,
            col: position.column as u32,
        }
    }
}

fn exported(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// Index of the `]` that closes a `[` just before `text`.
fn matching_bracket(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (ix, ch) in text.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' if depth == 0 => return Some(ix),
            ']' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn find_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| find_kind(child, kind))
}

// ---------------------------------------------------------------- queries

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

fn word_start(src: &str, offset: usize) -> usize {
    let bytes = src.as_bytes();
    let mut start = offset.min(bytes.len());
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    start
}

fn word_end(src: &str, offset: usize) -> usize {
    let bytes = src.as_bytes();
    let mut end = offset.min(bytes.len());
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    end
}

/// Where the expression ending just before the `.` at `dot` starts.
fn receiver_start(src: &str, dot: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut at = dot;
    loop {
        // Calls and index expressions: skip back over the bracketed group.
        while at > 0 && matches!(bytes[at - 1], b')' | b']') {
            let close = bytes[at - 1];
            let open = if close == b')' { b'(' } else { b'[' };
            let mut depth = 0usize;
            let mut ix = at;
            loop {
                if ix == 0 {
                    return None;
                }
                ix -= 1;
                if bytes[ix] == close {
                    depth += 1;
                } else if bytes[ix] == open {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            at = ix;
        }
        let end = at;
        while at > 0 && is_ident(bytes[at - 1]) {
            at -= 1;
        }
        if at == end {
            return None;
        }
        if at > 0 && bytes[at - 1] == b'.' {
            at -= 1;
        } else {
            break;
        }
    }
    (!bytes[at].is_ascii_digit()).then_some(at)
}

const KEYWORDS: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
    "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
    "return", "select", "struct", "switch", "type", "var",
];

const BUILTINS: &[&str] = &[
    "append", "cap", "clear", "close", "complex", "copy", "delete", "imag", "len", "make", "max",
    "min", "new", "panic", "print", "println", "real", "recover", "any", "bool", "byte",
    "comparable", "complex128", "complex64", "error", "float32", "float64", "int", "int16",
    "int32", "int64", "int8", "rune", "string", "uint", "uint16", "uint32", "uint64", "uint8",
    "uintptr", "true", "false", "nil", "iota",
];

/// How well `name` answers `query`: lower is better, `None` is no match.
fn rank(name: &str, query: &str) -> Option<u8> {
    if name.starts_with(query) {
        return Some(0);
    }
    let lower = name.to_lowercase();
    let query = query.to_lowercase();
    if lower.starts_with(&query) {
        return Some(1);
    }
    if query.len() < 2 {
        return None;
    }
    if lower.contains(&query) {
        return Some(2);
    }
    if query.len() < 3 {
        return None;
    }
    // The letters in order, as in `nwr` for `NewReader`.
    let mut chars = lower.chars();
    query
        .chars()
        .all(|q| chars.any(|c| c == q))
        .then_some(3)
}

fn in_text_or_comment(tree: &Tree, start: usize, offset: usize) -> bool {
    let at = start.min(offset).saturating_sub(1);
    let mut node = tree.root_node().descendant_for_byte_range(at, at);
    while let Some(current) = node {
        if matches!(
            current.kind(),
            "comment"
                | "interpreted_string_literal"
                | "raw_string_literal"
                | "rune_literal"
                | "interpreted_string_literal_content"
                | "raw_string_literal_content"
        ) {
            // The closing quote ends the literal: text right after it is code.
            return current.end_byte() > at + 1 || current.kind() == "comment";
        }
        node = current.parent();
    }
    false
}

/// What can be typed at `offset` in `source`, the editor's text of `file`.
pub fn complete(index: &Index, file: &Path, source: &str, offset: usize) -> Completions {
    let offset = offset.min(source.len());
    let start = word_start(source, offset);
    let query = &source[start..offset];
    let empty = Completions {
        start,
        items: Vec::new(),
    };
    if query.starts_with(|c: char| c.is_ascii_digit()) {
        return empty;
    }
    let Some(tree) = parse(source) else {
        return empty;
    };
    if in_text_or_comment(&tree, start, offset) {
        return empty;
    }
    let before = &source[..start];
    let after_dot = before.ends_with('.');

    let scope = Scope::new(index, file, source, &tree, offset);
    let candidates: Vec<Symbol> = if after_dot {
        let dot = start - 1;
        let Some(receiver_start) = receiver_start(source, dot) else {
            return empty;
        };
        let mut members = scope.members_of(&source[receiver_start..dot]);
        // Fields first, then everything else, each by name.
        members.sort_by(|a, b| {
            (a.kind != Kind::Field, &a.name).cmp(&(b.kind != Kind::Field, &b.name))
        });
        members
    } else {
        if query.is_empty() {
            return empty;
        }
        // The name of a new declaration is not something to complete.
        let previous = before.trim_end().rsplit(|c: char| !c.is_alphanumeric()).next();
        if matches!(
            previous,
            Some("func" | "var" | "const" | "type" | "package" | "import")
        ) && before.ends_with(char::is_whitespace)
        {
            return empty;
        }
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for local in scope.locals.iter().rev() {
            if rank(local.name, query).is_some() && seen.insert(local.name.to_string()) {
                candidates.push(scope.local_symbol(local));
            }
        }
        for symbol in scope.package.symbols.iter().filter(|s| s.owner.is_none()) {
            if seen.insert(symbol.name.clone()) {
                candidates.push(symbol.clone());
            }
        }
        let plain = |name: &str, kind: Kind, detail: &str| Symbol {
            name: name.to_string(),
            kind,
            detail: detail.to_string(),
            owner: None,
            ty: None,
            embedded: false,
            file: scope.file.clone(),
            row: 0,
            col: 0,
        };
        for import in &scope.imports {
            if !matches!(import.name.as_str(), "_" | ".") && seen.insert(import.name.clone()) {
                candidates.push(plain(&import.name, Kind::Package, &import.path));
            }
        }
        for name in BUILTINS {
            if seen.insert(name.to_string()) {
                candidates.push(plain(name, Kind::Builtin, ""));
            }
        }
        for name in KEYWORDS {
            if seen.insert(name.to_string()) {
                candidates.push(plain(name, Kind::Keyword, ""));
            }
        }
        candidates
    };

    let mut ranked: Vec<(u8, Symbol)> = candidates
        .into_iter()
        .filter_map(|symbol| {
            let rank = if query.is_empty() {
                0
            } else {
                rank(&symbol.name, query)?
            };
            // Keywords and builtins are only meant when typed from their start.
            if rank > 1 && matches!(symbol.kind, Kind::Keyword | Kind::Builtin) {
                return None;
            }
            Some((rank, symbol))
        })
        .collect();
    // Stable: within a rank, the nearest declarations stay first.
    ranked.sort_by_key(|(rank, _)| *rank);
    // Typing a whole name that is the only candidate needs no menu.
    if ranked.len() == 1 && ranked[0].1.name == query {
        return empty;
    }
    Completions {
        start,
        items: ranked
            .into_iter()
            .take(MAX_ITEMS)
            .map(|(_, symbol)| Item {
                name: symbol.name,
                kind: symbol.kind,
                detail: symbol.detail,
            })
            .collect(),
    }
}

/// Where the name at `offset` is declared.
pub fn definition(index: &Index, file: &Path, source: &str, offset: usize) -> Option<Location> {
    let offset = offset.min(source.len());
    let start = word_start(source, offset);
    let end = word_end(source, offset);
    if start == end {
        return None;
    }
    let tree = parse(source)?;
    definition_in(index, file, source, &tree, offset)
}

/// [`definition`] in a file that has already been parsed.
fn definition_in(
    index: &Index,
    file: &Path,
    source: &str,
    tree: &Tree,
    offset: usize,
) -> Option<Location> {
    let offset = offset.min(source.len());
    let start = word_start(source, offset);
    let end = word_end(source, offset);
    if start == end {
        return None;
    }
    let name = &source[start..end];
    let scope = Scope::new(index, file, source, tree, start);
    let node = tree.root_node().descendant_for_byte_range(start, end)?;
    let parent = node.parent();

    let location = |symbol: &Symbol| Location {
        file: symbol.file.to_path_buf(),
        row: symbol.row,
        col: symbol.col,
    };

    if let Some(parent) = parent {
        match parent.kind() {
            "selector_expression"
                if parent.child_by_field_name("field").map(|f| f.id()) == Some(node.id()) =>
            {
                let operand = parent.child_by_field_name("operand")?;
                return scope
                    .member(operand, source, name, MAX_DEPTH)
                    .map(|symbol| location(&symbol));
            }
            "qualified_type"
                if parent.child_by_field_name("name").map(|n| n.id()) == Some(node.id()) =>
            {
                let package = text(parent.child_by_field_name("package")?, source);
                let dir = scope.import_dir(&scope.file, package)?;
                return scope
                    .package_symbol(&dir, name)
                    .map(|symbol| location(&symbol));
            }
            // A struct literal key names a field of the literal's type.
            "keyed_element" | "literal_element"
                if node.kind() == "field_identifier" || parent.kind() == "literal_element" =>
            {
                let mut ancestor = parent.parent();
                while let Some(current) = ancestor {
                    if current.kind() == "composite_literal" {
                        let ty = scope.ty(text(current.child_by_field_name("type")?, source));
                        if let Some(member) = scope
                            .members(&ty, MAX_DEPTH)
                            .into_iter()
                            .find(|member| member.name == name)
                        {
                            return Some(location(&member));
                        }
                        break;
                    }
                    ancestor = current.parent();
                }
            }
            _ => {}
        }
    }

    if node.kind() == "identifier"
        && let Some(local) = scope.local(name)
    {
        // The declaration itself has nowhere further to go.
        if local.node.start_byte() == start {
            return None;
        }
        return Some(location(&scope.local_symbol(local)));
    }
    let symbol = scope.package_symbol(&scope.dir, name)?;
    (!(symbol.file == scope.file && symbol.row == node.start_position().row as u32
        && symbol.col == node.start_position().column as u32))
        .then(|| location(&symbol))
}

// ---------------------------------------------------------------- usages

/// A place a name is used, or a type or method that implements an interface.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub file: PathBuf,
    pub row: u32,
    /// Byte column of the name within its line.
    pub col: u32,
    /// The line, trimmed, for display.
    pub line: String,
}

/// Everything that refers to one declaration.
#[derive(Debug, Default)]
pub struct Usages {
    pub name: String,
    /// For an interface or one of its methods: the types or methods that implement it.
    pub implementations: Vec<Hit>,
    pub usages: Vec<Hit>,
}

/// Node kinds a name can be written as.
const NAME_KINDS: &[&str] = &["identifier", "field_identifier", "type_identifier"];

/// Where the name at `offset` is declared, when it is that declaration itself.
pub fn declaration_at(index: &Index, file: &Path, source: &str, offset: usize) -> Option<Location> {
    let offset = offset.min(source.len());
    let (start, end) = (word_start(source, offset), word_end(source, offset));
    if start == end {
        return None;
    }
    let tree = parse(source)?;
    declaration_in(index, file, source, &tree, start, end)
}

fn declaration_in(
    index: &Index,
    file: &Path,
    source: &str,
    tree: &Tree,
    start: usize,
    end: usize,
) -> Option<Location> {
    let node = tree.root_node().descendant_for_byte_range(start, end)?;
    if !NAME_KINDS.contains(&node.kind()) || node.start_byte() != start {
        return None;
    }
    let position = node.start_position();
    let (row, col) = (position.row as u32, position.column as u32);
    let here = Location {
        file: file.to_path_buf(),
        row,
        col,
    };
    let (_, symbols) = file_symbols(source, tree, &Arc::from(file));
    if symbols.iter().any(|s| s.row == row && s.col == col) {
        return Some(here);
    }
    let scope = Scope::new(index, file, source, tree, end);
    scope
        .locals
        .iter()
        .any(|local| local.node.start_byte() == start)
        .then_some(here)
}

/// Whether `word` occurs in `text` on its own, not as part of a longer name.
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(word).any(|(ix, _)| {
        let end = ix + word.len();
        (ix == 0 || !is_ident(bytes[ix - 1])) && (end >= bytes.len() || !is_ident(bytes[end]))
    })
}

/// The byte offset of `row` and byte column `col` in `src`.
fn offset_of(src: &str, row: u32, col: u32) -> usize {
    let line_start: usize = src
        .split_inclusive('\n')
        .take(row as usize)
        .map(str::len)
        .sum();
    (line_start + col as usize).min(src.len())
}

fn hit(file: &Path, src: &str, row: u32, col: u32) -> Hit {
    let mut line = src
        .lines()
        .nth(row as usize)
        .unwrap_or_default()
        .trim()
        .to_string();
    if line.len() > 160 {
        let mut end = 157;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
        line.push('…');
    }
    Hit {
        file: file.to_path_buf(),
        row,
        col,
        line,
    }
}

/// Every use of what the name at `offset` declares or refers to, looked for in
/// `files`; `open` holds the editor's text of files that are open.
///
/// Uses are found by name and then resolved one by one, so a use is listed
/// only when it leads back to the same declaration.
pub fn usages(
    index: &Index,
    files: &[PathBuf],
    open: &HashMap<PathBuf, String>,
    file: &Path,
    source: &str,
    offset: usize,
) -> Option<Usages> {
    let offset = offset.min(source.len());
    let (start, end) = (word_start(source, offset), word_end(source, offset));
    if start == end {
        return None;
    }
    let name = source[start..end].to_string();
    let tree = parse(source)?;
    let target = definition_in(index, file, source, &tree, offset)
        .or_else(|| declaration_in(index, file, source, &tree, start, end))?;

    let read = |path: &Path| -> Option<String> {
        if path == file {
            return Some(source.to_string());
        }
        open.get(path)
            .cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
    };
    let target_src = read(&target.file)?;
    let target_tree = parse(&target_src)?;
    let (_, symbols) = file_symbols(&target_src, &target_tree, &Arc::from(target.file.as_path()));
    let symbol = symbols
        .iter()
        .find(|s| s.row == target.row && s.col == target.col);
    let target_dir = target.file.parent().map(Path::to_path_buf).unwrap_or_default();

    // A local is only seen inside the declaration around it.
    let mut within = None;
    let mut candidates: Vec<PathBuf> = vec![target.file.clone()];
    match symbol {
        None => {
            let at = offset_of(&target_src, target.row, target.col);
            let root = target_tree.root_node();
            let mut cursor = root.walk();
            within = root
                .children(&mut cursor)
                .find(|node| node.start_byte() <= at && at < node.end_byte())
                .map(|node| node.byte_range());
        }
        Some(symbol) => {
            let in_package_only = !exported(&symbol.name);
            candidates.extend(
                files
                    .iter()
                    .filter(|path| path.extension().is_some_and(|ext| ext == "go"))
                    .filter(|path| !in_package_only || path.parent() == Some(target_dir.as_path()))
                    .cloned(),
            );
        }
    }
    candidates.sort();
    candidates.dedup();

    let mut out = Usages {
        name: name.clone(),
        ..Default::default()
    };
    for path in &candidates {
        let content = if *path == target.file {
            Some(target_src.clone())
        } else {
            read(path)
        };
        let Some(content) = content else {
            continue;
        };
        if !contains_word(&content, &name) {
            continue;
        }
        let Some(tree) = parse(&content) else {
            continue;
        };
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if let Some(range) = &within
                && (node.end_byte() <= range.start || node.start_byte() >= range.end)
            {
                continue;
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
            if !NAME_KINDS.contains(&node.kind()) || text(node, &content) != name {
                continue;
            }
            let position = node.start_position();
            let (row, col) = (position.row as u32, position.column as u32);
            if *path == target.file && row == target.row && col == target.col {
                continue;
            }
            let found = definition_in(index, path, &content, &tree, node.start_byte());
            if found.as_ref() == Some(&target) {
                out.usages.push(hit(path, &content, row, col));
            }
        }
    }
    out.usages
        .sort_by(|a, b| (&a.file, a.row, a.col).cmp(&(&b.file, b.row, b.col)));
    out.usages.dedup();

    if let Some(symbol) = symbol {
        let is_interface = |name: &str| {
            symbols
                .iter()
                .any(|s| s.kind == Kind::Type && s.name == name && s.detail == "interface")
        };
        let interface = match (symbol.kind, &symbol.owner) {
            (Kind::Type, None) if symbol.detail == "interface" => {
                Some((symbol.name.as_str(), None))
            }
            (Kind::Method, Some(owner)) if is_interface(owner) => {
                Some((owner.as_str(), Some(symbol.name.as_str())))
            }
            _ => None,
        };
        if let Some((interface, method)) = interface {
            out.implementations = implementations(index, files, &target.file, interface, method);
        }
    }
    Some(out)
}

/// How many parameters and results a signature has; `None` when it was cut short.
fn arity(signature: &str) -> Option<(usize, usize)> {
    if signature.ends_with('…') {
        return None;
    }
    let rest = signature.strip_prefix("func")?.trim_start();
    // The length of the bracketed group `text` starts with, and its item count.
    let group = |text: &str| -> Option<(usize, usize)> {
        let mut depth = 0usize;
        let mut commas = 0usize;
        let mut empty = true;
        for (ix, ch) in text.char_indices() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some((ix + 1, if empty { 0 } else { commas + 1 }));
                    }
                }
                ',' if depth == 1 => commas += 1,
                ch if !ch.is_whitespace() && depth >= 1 => empty = false,
                _ => {}
            }
        }
        None
    };
    let (length, parameters) = group(rest)?;
    let results = rest[length..].trim();
    let results = if results.is_empty() {
        0
    } else if results.starts_with('(') {
        group(results)?.1
    } else {
        1
    };
    Some((parameters, results))
}

/// The types in the directories of `files` that have every method of the
/// interface `interface` declared in `file`; with `method`, that method of each.
fn implementations(
    index: &Index,
    files: &[PathBuf],
    file: &Path,
    interface: &str,
    method: Option<&str>,
) -> Vec<Hit> {
    let Some(dir) = file.parent() else {
        return Vec::new();
    };
    let interface_ty = Ty {
        text: interface.to_string(),
        file: Arc::from(file),
    };
    let wanted: Vec<(String, Option<(usize, usize)>)> = Scope::for_dir(index, dir)
        .members(&interface_ty, MAX_DEPTH)
        .into_iter()
        .filter(|member| member.kind == Kind::Method)
        .map(|member| (member.name.clone(), arity(&member.detail)))
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }

    let mut dirs: Vec<&Path> = files
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "go"))
        .filter_map(|path| path.parent())
        .collect();
    dirs.push(dir);
    dirs.sort();
    dirs.dedup();

    let mut out = Vec::new();
    for dir in dirs {
        let scope = Scope::for_dir(index, dir);
        let types = scope
            .package
            .symbols
            .iter()
            .filter(|s| s.kind == Kind::Type && s.owner.is_none() && s.detail != "interface");
        for ty in types {
            let members = scope.members(
                &Ty {
                    text: ty.name.clone(),
                    file: ty.file.clone(),
                },
                MAX_DEPTH,
            );
            let has = |(name, shape): &(String, Option<(usize, usize)>)| {
                members.iter().any(|member| {
                    member.kind == Kind::Method
                        && &member.name == name
                        && match (shape, arity(&member.detail)) {
                            (Some(wanted), Some(found)) => *wanted == found,
                            _ => true,
                        }
                })
            };
            if !wanted.iter().all(has) {
                continue;
            }
            let at = match method {
                Some(method) => members
                    .iter()
                    .find(|member| member.kind == Kind::Method && member.name == method),
                None => Some(ty),
            };
            if let Some(at) = at {
                let src = std::fs::read_to_string(&at.file).unwrap_or_default();
                out.push(hit(&at.file, &src, at.row, at.col));
            }
        }
    }
    out.sort_by(|a, b| (&a.file, a.row).cmp(&(&b.file, b.row)));
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module on disk: `files` are `(relative path, content)`.
    struct Fixture {
        root: PathBuf,
        index: Index,
    }

    impl Fixture {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let root = std::env::temp_dir().join(format!("gust-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for (path, content) in files {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, content).unwrap();
            }
            Self {
                root,
                index: Index::new(),
            }
        }

        /// Complete at the `|` in `source`, edited in place of `file`.
        fn complete(&self, file: &str, source: &str) -> Vec<String> {
            let offset = source.find('|').expect("the source marks the cursor with |");
            let source = source.replacen('|', "", 1);
            complete(&self.index, &self.root.join(file), &source, offset)
                .items
                .into_iter()
                .map(|item| item.name)
                .collect()
        }

        fn definition(&self, file: &str, source: &str) -> Option<(String, u32)> {
            let offset = source.find('|').expect("the source marks the cursor with |");
            let source = source.replacen('|', "", 1);
            definition(&self.index, &self.root.join(file), &source, offset).map(|location| {
                let file = location
                    .file
                    .strip_prefix(&self.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                (file, location.row)
            })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const GO_MOD: (&str, &str) = ("go.mod", "module example.com/demo\n\ngo 1.22\n");

    #[test]
    fn names_declared_above_the_cursor_are_offered() {
        let fixture = Fixture::new("locals", &[GO_MOD]);
        let items = fixture.complete(
            "main.go",
            "package main\n\nconst version = \"0.1.0\"\n\nfunc verify(token string) bool {\n\treturn token != \"\"\n}\n\nfunc main() {\n\tvalue := 1\n\tprintln(v|\n}\n",
        );
        assert_eq!(items, ["value", "version", "verify", "var"]);
    }

    #[test]
    fn names_from_a_closed_block_are_not_offered() {
        let fixture = Fixture::new("blocks", &[GO_MOD]);
        let items = fixture.complete(
            "main.go",
            "package main\n\nfunc main() {\n\tif true {\n\t\tinner := 1\n\t\t_ = inner\n\t}\n\touter := 2\n\t_ = outer\n\tin|\n}\n",
        );
        assert!(!items.contains(&"inner".to_string()), "{items:?}");
        let items = fixture.complete(
            "main.go",
            "package main\n\nfunc main() {\n\touter := 2\n\tif true {\n\t\tou|\n\t}\n}\n",
        );
        assert_eq!(items, ["outer"]);
    }

    #[test]
    fn other_files_of_the_package_are_seen() {
        let fixture = Fixture::new(
            "siblings",
            &[
                GO_MOD,
                ("util.go", "package main\n\nfunc helperOne() {}\n\ntype helperType struct{}\n"),
            ],
        );
        let items = fixture.complete("main.go", "package main\n\nfunc main() {\n\thelp|\n}\n");
        assert_eq!(items, ["helperOne", "helperType"]);
    }

    #[test]
    fn fields_and_methods_follow_a_local_through_its_constructor() {
        let fixture = Fixture::new(
            "members",
            &[
                GO_MOD,
                (
                    "store/store.go",
                    "package store\n\ntype Base struct {\n\tID int\n}\n\ntype Store struct {\n\tBase\n\tName string\n\tsecret string\n}\n\nfunc New() (*Store, error) { return nil, nil }\n\nfunc (s *Store) Save() error { return nil }\n\nfunc (s *Store) hidden() {}\n",
                ),
            ],
        );
        let source = "package main\n\nimport \"example.com/demo/store\"\n\nfunc main() {\n\ts, err := store.New()\n\t_ = err\n\ts.|\n}\n";
        assert_eq!(
            fixture.complete("main.go", source),
            ["Base", "ID", "Name", "Save"]
        );

        let package = "package main\n\nimport \"example.com/demo/store\"\n\nfunc main() {\n\tstore.N|\n}\n";
        assert_eq!(fixture.complete("main.go", package), ["New"]);
    }

    #[test]
    fn members_of_the_same_package_include_unexported_ones() {
        let fixture = Fixture::new("own", &[GO_MOD]);
        let source = "package main\n\ntype config struct {\n\thost string\n\tport int\n}\n\nfunc (c *config) addr() string { return c.host }\n\nfunc load() *config { return nil }\n\nfunc main() {\n\tcfg := load()\n\tcfg.|\n}\n";
        assert_eq!(fixture.complete("main.go", source), ["host", "port", "addr"]);

        let ranged = "package main\n\ntype item struct{ title string }\n\nfunc main() {\n\titems := []item{}\n\tfor _, it := range items {\n\t\tit.t|\n\t}\n}\n";
        assert_eq!(fixture.complete("main.go", ranged), ["title"]);
    }

    #[test]
    fn nothing_is_offered_inside_strings_and_comments() {
        let fixture = Fixture::new("strings", &[GO_MOD]);
        let source = "package main\n\nvar value = 1\n\nfunc main() {\n\tprintln(\"va|\")\n}\n";
        assert!(fixture.complete("main.go", source).is_empty());
        let comment = "package main\n\nvar value = 1\n\n// va|\nfunc main() {}\n";
        assert!(fixture.complete("main.go", comment).is_empty());
    }

    #[test]
    fn the_standard_library_is_read_from_goroot() {
        if go_env().goroot.is_none() {
            return;
        }
        let fixture = Fixture::new("std", &[GO_MOD]);
        let source = "package main\n\nimport \"strings\"\n\nfunc main() {\n\tstrings.HasP|\n}\n";
        assert_eq!(fixture.complete("main.go", source), ["HasPrefix"]);

        let builder = "package main\n\nimport \"strings\"\n\nfunc main() {\n\tvar b strings.Builder\n\tb.WriteS|\n}\n";
        assert_eq!(fixture.complete("main.go", builder), ["WriteString"]);
    }

    /// Runs against whatever this machine has downloaded; skipped without it.
    #[test]
    fn dependencies_are_read_from_the_module_cache() {
        let cached = |module: &str| {
            go_env()
                .modcache
                .as_ref()
                .is_some_and(|cache| cache.join(module).is_dir())
        };
        if !cached("go.uber.org/zap@v1.27.1") {
            return;
        }
        let fixture = Fixture::new(
            "deps",
            &[(
                "go.mod",
                "module example.com/demo\n\ngo 1.22\n\nrequire (\n\tgo.uber.org/zap v1.27.1\n\tgithub.com/redis/go-redis/v9 v9.17.0\n)\n",
            )],
        );
        let source = "package main\n\nimport \"go.uber.org/zap\"\n\nfunc main() {\n\tlogger := zap.NewNop()\n\tlogger.Sug|\n}\n";
        assert_eq!(fixture.complete("main.go", source), ["Sugar"]);

        if cached("github.com/redis/go-redis/v9@v9.17.0") {
            // The package is named `redis`, not `v9` or `go-redis`.
            let source = "package main\n\nimport \"github.com/redis/go-redis/v9\"\n\nfunc main() {\n\tredis.NewCli|\n}\n";
            assert_eq!(fixture.complete("main.go", source), ["NewClient"]);
        }
    }

    #[test]
    fn definitions_are_found_across_files_and_packages() {
        let fixture = Fixture::new(
            "definitions",
            &[
                GO_MOD,
                ("util.go", "package main\n\nfunc helper() {}\n"),
                (
                    "store/store.go",
                    "package store\n\ntype Store struct {\n\tName string\n}\n\nfunc New() *Store { return nil }\n",
                ),
            ],
        );
        let source = "package main\n\nimport \"example.com/demo/store\"\n\nfunc main() {\n\tcount := 1\n\thel|per()\n\ts := store.Ne|w()\n\t_ = s.Na|me\n\t_ = cou|nt\n}\n";
        let at = |marker: &str| {
            // Keep only the marker on the wanted name.
            let keep = source.find(marker).unwrap() + marker.find('|').unwrap();
            let mut out = String::new();
            for (ix, ch) in source.char_indices() {
                if ch != '|' || ix == keep {
                    out.push(ch);
                }
            }
            fixture.definition("main.go", &out)
        };
        assert_eq!(at("hel|per"), Some(("util.go".to_string(), 2)));
        assert_eq!(at("Ne|w"), Some(("store/store.go".to_string(), 6)));
        assert_eq!(at("Na|me"), Some(("store/store.go".to_string(), 3)));
        assert_eq!(at("cou|nt"), Some(("main.go".to_string(), 5)));
    }

    #[test]
    fn go_mod_requirements_and_replacements_are_read() {
        let module = parse_go_mod(
            Path::new("/m"),
            "module example.com/demo // comment\n\ngo 1.22\n\nrequire (\n\tgithub.com/a/b v1.2.3\n\tgithub.com/c/d v0.1.0 // indirect\n)\n\nrequire github.com/e/f v2.0.0\n\nreplace github.com/a/b => ../b\n",
        );
        assert_eq!(module.path, "example.com/demo");
        assert_eq!(module.requires.len(), 3);
        assert_eq!(module.requires[2], ("github.com/e/f".into(), "v2.0.0".into()));
        assert_eq!(
            module.replaces,
            [("github.com/a/b".to_string(), "../b".to_string(), None)]
        );
    }

    #[test]
    fn import_paths_name_their_package() {
        assert_eq!(default_package_name("net/http"), "http");
        assert_eq!(default_package_name("example.com/mod/v2"), "mod");
        assert_eq!(default_package_name("gopkg.in/yaml.v3"), "yaml");
        assert_eq!(default_package_name("github.com/x/go-redis"), "redis");
    }

    impl Fixture {
        /// Usages of the name at the `|` in the fixture's `file`, as
        /// `(relative path, row)` pairs: implementations, then uses.
        fn usages(&self, file: &str, marker: &str) -> (Vec<(String, u32)>, Vec<(String, u32)>) {
            let path = self.root.join(file);
            let source = std::fs::read_to_string(&path).unwrap();
            let name = marker.replace('|', "");
            let offset = source.find(&name).expect("the marker is in the file") + marker.find('|').unwrap();
            let files: Vec<PathBuf> = crate::finder::project_files(&self.root, &[])
                .into_iter()
                .map(|relative| self.root.join(relative))
                .collect();
            let found = usages(&self.index, &files, &HashMap::new(), &path, &source, offset)
                .expect("the name resolves");
            let relative = |hits: Vec<Hit>| -> Vec<(String, u32)> {
                hits.into_iter()
                    .map(|hit| {
                        let file = hit.file.strip_prefix(&self.root).unwrap();
                        (file.to_string_lossy().replace('\\', "/"), hit.row)
                    })
                    .collect()
            };
            (relative(found.implementations), relative(found.usages))
        }
    }

    fn at(file: &str, row: u32) -> (String, u32) {
        (file.to_string(), row)
    }

    #[test]
    fn usages_of_a_function_are_found_across_packages_and_tests() {
        let fixture = Fixture::new(
            "usages-func",
            &[
                GO_MOD,
                ("store/store.go", "package store\n\nfunc Open() int { return 1 }\n\nfunc other() {\n\tOpen()\n}\n"),
                ("store/store_test.go", "package store\n\nfunc TestOpen() {\n\tOpen()\n}\n"),
                (
                    "main.go",
                    "package main\n\nimport \"example.com/demo/store\"\n\n// Open is mentioned in a comment.\nfunc Open() {}\n\nfunc main() {\n\tstore.Open()\n\tOpen()\n}\n",
                ),
            ],
        );
        // From the declaration: only uses of `store.Open`, not `main.Open`.
        let (_, uses) = fixture.usages("store/store.go", "func |Open");
        assert_eq!(uses, [at("main.go", 8), at("store/store.go", 5), at("store/store_test.go", 3)]);
        // From a use, the same answer.
        let (_, uses) = fixture.usages("main.go", "store.Op|en");
        assert_eq!(uses, [at("main.go", 8), at("store/store.go", 5), at("store/store_test.go", 3)]);
    }

    #[test]
    fn usages_of_a_local_stay_inside_its_function() {
        let fixture = Fixture::new(
            "usages-local",
            &[
                GO_MOD,
                (
                    "main.go",
                    "package main\n\nfunc a() {\n\tcount := 1\n\tcount++\n\t_ = count\n}\n\nfunc b() {\n\tcount := 2\n\t_ = count\n}\n",
                ),
            ],
        );
        let (implementations, uses) = fixture.usages("main.go", "cou|nt := 1");
        assert!(implementations.is_empty());
        assert_eq!(uses, [at("main.go", 4), at("main.go", 5)]);
    }

    #[test]
    fn usages_of_a_field_include_literal_keys() {
        let fixture = Fixture::new(
            "usages-field",
            &[
                GO_MOD,
                (
                    "main.go",
                    "package main\n\ntype Room struct {\n\tChips int\n}\n\nfunc main() {\n\tr := Room{Chips: 1}\n\tr.Chips += 2\n}\n",
                ),
            ],
        );
        let (_, uses) = fixture.usages("main.go", "Ch|ips int");
        assert_eq!(uses, [at("main.go", 7), at("main.go", 8)]);
    }

    #[test]
    fn implementations_of_an_interface_are_found_by_their_methods() {
        let wallet = "package port\n\ntype Wallet interface {\n\tBalance(id string) (int64, error)\n\tName() string\n}\n";
        let fixture = Fixture::new(
            "implementations",
            &[
                GO_MOD,
                ("port/port.go", wallet),
                (
                    "infra/client.go",
                    "package infra\n\ntype Client struct{}\n\nfunc (c *Client) Balance(id string) (int64, error) { return 0, nil }\n\nfunc (c Client) Name() string { return \"\" }\n",
                ),
                // A method with another shape does not implement it.
                (
                    "infra/other.go",
                    "package infra\n\ntype Other struct{}\n\nfunc (Other) Balance() int64 { return 0 }\n\nfunc (Other) Name() string { return \"\" }\n",
                ),
                // Methods can come from an embedded type.
                (
                    "infra/wrapped.go",
                    "package infra\n\ntype Wrapped struct {\n\t*Client\n}\n",
                ),
                (
                    "game/fake_test.go",
                    "package game\n\ntype fake struct{}\n\nfunc (fake) Balance(string) (int64, error) { return 0, nil }\nfunc (fake) Name() string { return \"\" }\n",
                ),
            ],
        );
        let (implementations, _) = fixture.usages("port/port.go", "type |Wallet");
        assert_eq!(
            implementations,
            [at("game/fake_test.go", 2), at("infra/client.go", 2), at("infra/wrapped.go", 2)]
        );
        let (implementations, _) = fixture.usages("port/port.go", "|Balance(id");
        assert_eq!(
            implementations,
            [at("game/fake_test.go", 4), at("infra/client.go", 4)]
        );
    }

    #[test]
    fn a_declaration_is_told_apart_from_a_use() {
        let fixture = Fixture::new("declaration", &[GO_MOD]);
        let source = "package main\n\nfunc run(n int) {\n\tx := n\n\t_ = x\n}\n";
        let file = fixture.root.join("main.go");
        let declared = |marker: &str| {
            let offset = source.find(marker).unwrap();
            declaration_at(&fixture.index, &file, source, offset).map(|location| location.row)
        };
        assert_eq!(declared("run"), Some(2));
        assert_eq!(declared("n int"), Some(2));
        assert_eq!(declared("x :="), Some(3));
        assert_eq!(declared("x\n}"), None);
        assert_eq!(declared("n\n"), None);
    }

    #[test]
    fn signatures_are_counted_by_parameters_and_results() {
        assert_eq!(arity("func()"), Some((0, 0)));
        assert_eq!(arity("func(a, b int) error"), Some((2, 1)));
        assert_eq!(arity("func(m map[string]int, f func(int, int)) (int, error)"), Some((2, 2)));
        assert_eq!(arity("func(a int) (n int, err error)"), Some((1, 2)));
        assert_eq!(arity("func(a, b, c, d…"), None);
    }
}
