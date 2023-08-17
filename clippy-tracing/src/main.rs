#![warn(clippy::pedantic)]

use clap::{Parser, ValueEnum};
use std::collections::HashMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use std::error::Error;

#[derive(Parser)]
struct CommandLineArgs {
    /// The action to take.
    #[arg(long)]
    action: Action,
    /// The path to look in.
    #[arg(long)]
    path: Option<PathBuf>,
    /// Sub-paths which contain any of the strings from this list will be ignored.
    #[arg(long, value_delimiter = ',')]
    exclude: Vec<String>,
}
#[derive(Clone, ValueEnum)]
enum Action {
    /// Checks `tracing::instrument` is on all functions.
    Check,
    /// Adds `tracing::instrument` to all functions.
    Fix,
    /// Removes `tracing::instrument` from all functions.
    Strip,
}

struct SegmentedList {
    first: String,
    inner: Vec<(String, String)>,
}
impl SegmentedList {
    fn set_before(&mut self, line: usize, text: String) {
        let s = if let Some(i) = line.checked_sub(1) {
            &mut self.inner[i].1
        } else {
            &mut self.first
        };
        debug_assert!(s.is_empty());
        *s = text;
    }
}
impl From<SegmentedList> for String {
    fn from(list: SegmentedList) -> String {
        let iter = list
            .inner
            .into_iter()
            .map(|(x, y)| format!("{x}{}{y}", if y.is_empty() { "" } else { "\n" }));
        format!(
            "{}{}{}",
            list.first,
            if list.first.is_empty() { "" } else { "\n" },
            itertools::intersperse(iter, String::from("\n")).collect::<String>()
        )
    }
}

struct StripVisitor(HashMap<usize, String>);
impl From<StripVisitor> for String {
    fn from(visitor: StripVisitor) -> String {
        let mut vec = visitor.0.into_iter().collect::<Vec<_>>();
        vec.sort_by_key(|(i, _)| *i);
        itertools::intersperse(vec.into_iter().map(|(_, x)| x), String::from("\n"))
            .collect::<String>()
    }
}
impl syn::visit::Visit<'_> for StripVisitor {
    fn visit_impl_item_fn(&mut self, i: &syn::ImplItemFn) {
        if let Some(instrument) = find_instrumented(&i.attrs) {
            let line = instrument.span().start().line - 1;
            self.0.remove(&line);
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        if let Some(instrument) = find_instrumented(&i.attrs) {
            let line = instrument.span().start().line - 1;
            self.0.remove(&line);
        }
        self.visit_block(&i.block);
    }
}

struct CheckVisitor(Option<proc_macro2::Span>);
impl syn::visit::Visit<'_> for CheckVisitor {
    fn visit_impl_item_fn(&mut self, i: &syn::ImplItemFn) {
        let attr = check_attributes(&i.attrs);
        if !attr.instrumented && !attr.skipped && !attr.test && i.sig.constness.is_none() {
            self.0 = Some(i.span());
        } else {
            self.visit_block(&i.block);
        }
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        let attr = check_attributes(&i.attrs);
        if !attr.instrumented && !attr.skipped && !attr.test && i.sig.constness.is_none() {
            self.0 = Some(i.span());
        } else {
            self.visit_block(&i.block);
        }
    }
}

struct FixVisitor(SegmentedList);
impl From<FixVisitor> for String {
    fn from(visitor: FixVisitor) -> String {
        String::from(visitor.0)
    }
}

impl syn::visit::Visit<'_> for FixVisitor {
    fn visit_impl_item_fn(&mut self, i: &syn::ImplItemFn) {
        let attr = check_attributes(&i.attrs);
        if !attr.instrumented && !attr.skipped && !attr.test && i.sig.constness.is_none() {
            let line = i.span().start().line;

            let attr_string = instrument(&i.sig);
            let indent = i.span().start().column;
            let attr = format!("{}{attr_string}", " ".repeat(indent));
            self.0.set_before(line - 1, attr);
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        let attr = check_attributes(&i.attrs);

        if !attr.instrumented && !attr.skipped && !attr.test && i.sig.constness.is_none() {
            let line = i.span().start().line;

            let attr_string = instrument(&i.sig);
            let indent = i.span().start().column;
            let attr = format!("{}{attr_string}", " ".repeat(indent));
            self.0.set_before(line - 1, attr);
        }
        self.visit_block(&i.block);
    }
}

fn instrument(sig: &syn::Signature) -> String {
    let iter = sig.inputs.iter().flat_map(|arg| match arg {
        syn::FnArg::Receiver(_) => vec![String::from("self")],
        syn::FnArg::Typed(syn::PatType { pat, .. }) => match &**pat {
            syn::Pat::Ident(syn::PatIdent { ident, .. }) => vec![ident.to_string()],
            syn::Pat::Struct(syn::PatStruct { fields, .. }) => fields
                .iter()
                .filter_map(|f| match &f.member {
                    syn::Member::Named(ident) => Some(ident.to_string()),
                    syn::Member::Unnamed(_) => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        },
    });
    let args = itertools::intersperse(iter, String::from(",")).collect::<String>();

    format!("#[tracing::instrument(level = \"trace\", skip({args}))]")
}

fn main() {
    if let Err(err) = exec() {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

#[derive(Debug)]
enum ExecError {
    Entry(walkdir::Error),
    String,
    Apply(ApplyError),
}
impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry(entry) => write!(f, "Failed to read entry in file path: {entry}"),
            Self::String => write!(f, "Failed to parse file path to string."),
            Self::Apply(apply) => write!(f, "Failed to run apply function: {apply}"),
        }
    }
}

impl Error for ExecError {}

fn exec() -> Result<(), ExecError> {
    let args = CommandLineArgs::parse();

    let path = args.path.unwrap_or(PathBuf::from("."));
    for entry_res in WalkDir::new(path).follow_links(true) {
        let entry = entry_res.map_err(ExecError::Entry)?;
        let path = entry.into_path();

        let path_str = path.to_str().ok_or(ExecError::String)?;
        // File paths must not contain any excluded strings.
        let a = !args.exclude.iter().any(|e| path_str.contains(e));
        // The file must not be a `build.rs` file.
        let b = !path.ends_with("build.rs");
        // The file must be a `.rs` file.
        let c = path.extension().map_or(false, |ext| ext == "rs");

        if a && b && c {
            let file = OpenOptions::new().read(true).open(&path).unwrap();
            let res = apply(&args.action, file, |_| {
                OpenOptions::new().write(true).truncate(true).open(&path)
            })
            .map_err(ExecError::Apply)?;

            if let Some(span) = res {
                println!(
                    "Missing instrumentation at {path_str}:{}:{}.",
                    span.start().line,
                    span.start().column
                );
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
enum ApplyError {
    Read(std::io::Error),
    Utf(std::str::Utf8Error),
    Syn(syn::parse::Error),
    Target(std::io::Error),
    Write(std::io::Error),
}
impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(read) => write!(f, "Failed to read file: {read}"),
            Self::Utf(utf) => write!(f, "Failed to parse file to utf8: {utf}"),
            Self::Syn(syn) => write!(f, "Failed to parse file to syn ast: {syn}"),
            Self::Target(target) => write!(f, "Failed to get write target: {target}"),
            Self::Write(write) => write!(f, "Failed to write result to target: {write}"),
        }
    }
}

impl Error for ApplyError {}

fn apply<R: Read, W: Write>(
    action: &Action,
    mut source: R,
    target: impl Fn(R) -> Result<W, std::io::Error>,
) -> Result<Option<proc_macro2::Span>, ApplyError> {
    let mut buf = Vec::new();
    source.read_to_end(&mut buf).map_err(ApplyError::Read)?;
    let text = std::str::from_utf8(&buf).map_err(ApplyError::Utf)?;

    let ast = syn::parse_file(text).map_err(ApplyError::Syn)?;

    match action {
        Action::Strip => {
            let mut visitor = StripVisitor(
                text.split('\n')
                    .enumerate()
                    .map(|(i, x)| (i, String::from(x)))
                    .collect(),
            );
            visitor.visit_file(&ast);
            let out = String::from(visitor);
            target(source)
                .map_err(ApplyError::Target)?
                .write_all(out.as_bytes())
                .map_err(ApplyError::Write)?;
            Ok(None)
        }
        Action::Check => {
            let mut visitor = CheckVisitor(None);
            visitor.visit_file(&ast);
            Ok(visitor.0)
        }
        Action::Fix => {
            let mut visitor = FixVisitor(SegmentedList {
                first: String::new(),
                inner: text
                    .split('\n')
                    .map(|x| (String::from(x), String::new()))
                    .collect(),
            });
            visitor.visit_file(&ast);
            let out = String::from(visitor);
            target(source)
                .map_err(ApplyError::Target)?
                .write_all(out.as_bytes())
                .map_err(ApplyError::Write)?;
            Ok(None)
        }
    }
}

// Finds the `#[instrument]` attribute on a function.
fn find_instrumented(attrs: &[syn::Attribute]) -> Option<&syn::Attribute> {
    attrs.iter().find(|attr| {
        match &attr.meta {
            syn::Meta::List(syn::MetaList { path, .. }) => matches!(path.segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument"),
            syn::Meta::Path(syn::Path { segments, .. }) => matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument"),
            syn::Meta::NameValue(_) => false,
        }
    })
}

struct Desc {
    instrumented: bool,
    skipped: bool,
    test: bool,
}

// A function is considered instruments if it has the `#[instrument]` attribute or the `#[test]`
// attribute.
/// Returns a tuple where the 1st element is whether `tracing::instrument` is found in the list of
/// attributes and the 2nd is whether `clippy_tracing_attributes::skip` is found in the list of
/// attributes.
fn check_attributes(attrs: &[syn::Attribute]) -> Desc {
    let mut instrumented = false;
    let mut skipped = false;
    let mut test = false;

    for attr in attrs {
        // Match `#[instrument]`.
        if match &attr.meta {
            syn::Meta::List(syn::MetaList { path, .. }) => {
                matches!(path.segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument")
            }
            syn::Meta::Path(syn::Path { segments, .. }) => {
                matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument")
            }
            syn::Meta::NameValue(_) => false,
        } {
            instrumented = true;
        }

        // Match `#[test]` or `#[kani::proof]`.
        if match &attr.meta {
            syn::Meta::List(syn::MetaList { path, .. }) => {
                matches!(path.segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "proof")
            }
            syn::Meta::Path(syn::Path { segments, .. }) => {
                matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "test" || ident == "proof")
            }
            syn::Meta::NameValue(_) => false,
        } {
            test = true;
        }

        // Match `#[clippy_tracing_skip]`.
        if match &attr.meta {
            syn::Meta::List(syn::MetaList { path, .. }) => {
                matches!(path.segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "clippy_tracing_skip")
            }
            syn::Meta::Path(syn::Path { segments, .. }) => {
                matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "clippy_tracing_skip")
            }
            syn::Meta::NameValue(_) => false,
        } {
            skipped = true;
        }
    }
    Desc {
        instrumented,
        skipped,
        test,
    }
}
