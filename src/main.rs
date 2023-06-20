#![warn(clippy::pedantic)]

use clap::{Args, Parser, Subcommand, ValueEnum};

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

// TODO When adding on `fmt` for `Display` always add `skip(f)`.
// TODO When adding on functions which return a mutable reference do not add `ret`.
// TODO Fix bug where it adds an extra newline each time you call `strip` or `fix`.

#[derive(Parser)]
struct CommandLineArgs {
    /// The path within which to work.
    #[command(subcommand)]
    input: Option<Input>,
    /// The action to take.
    #[arg(long)]
    action: Action,
}
#[derive(Subcommand)]
enum Input {
    /// Apply to a given text.
    Text(TextArgs),
    /// Apply to all files under the given path.
    Path(PathArgs),
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
#[derive(Args)]
struct TextArgs {
    /// The text to work on.
    #[arg(long)]
    text: String,
}
#[derive(Args)]
struct PathArgs {
    /// The path to look in.
    #[arg(long)]
    path: PathBuf,
    /// Sub-paths which contain any of the strings from this list will be ignored.
    #[arg(long)]
    exclude: Vec<String>,
}

struct SegmentedList {
    first: String,
    inner: Vec<(String, String)>,
}
impl SegmentedList {
    fn insert_before(&mut self, line: usize, text: &str) {
        if line == 0 {
            self.first.push_str(text);
        } else {
            self.inner[line - 1].1.push_str(text);
        }
    }
}
impl From<SegmentedList> for String {
    fn from(list: SegmentedList) -> String {
        format!(
            "{}\n{}",
            list.first,
            list.inner
                .into_iter()
                .map(|(x, y)| format!("{x}\n{y}"))
                .collect::<String>()
        )
    }
}

struct StripVisitor(HashMap<usize, String>);
impl From<StripVisitor> for String {
    fn from(visitor: StripVisitor) -> String {
        let mut vec = visitor.0.into_iter().collect::<Vec<_>>();
        vec.sort_by_key(|(i, _)| *i);
        vec.into_iter().map(|(_, x)| format!("{x}\n")).collect()
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

struct CheckVisitor(bool);
impl syn::visit::Visit<'_> for CheckVisitor {
    fn visit_impl_item_fn(&mut self, i: &syn::ImplItemFn) {
        if !is_instrumented(&i.attrs) && i.sig.constness.is_none() {
            self.0 = true;
        } else {
            self.visit_block(&i.block);
        }
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        if !is_instrumented(&i.attrs) && i.sig.constness.is_none() {
            self.0 = true;
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
        if !is_instrumented(&i.attrs) && i.sig.constness.is_none() {
            let line = i.sig.span().start().line;
            self.0.insert_before(line - 1, INSTRUMENT);
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        if !is_instrumented(&i.attrs) && i.sig.constness.is_none() {
            let line = i.sig.span().start().line;
            self.0.insert_before(line - 1, INSTRUMENT);
        }
        self.visit_block(&i.block);
    }
}

#[derive(Debug)]
enum Error {
    Check,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Check => write!(f, "Found a function missing instrumentation."),
        }
    }
}
impl std::error::Error for Error {}

const INSTRUMENT: &str = "#[tracing::instrument(level = \"trace\", ret)]";

use std::fs::OpenOptions;

fn main() -> Result<(), Error> {
    let args = CommandLineArgs::parse();

    let input = args.input.unwrap_or(Input::Path(PathArgs {
        path: PathBuf::from("."),
        exclude: Vec::new(),
    }));

    match input {
        Input::Path(PathArgs { path, exclude }) => {
            for entry_res in WalkDir::new(path).follow_links(true) {
                let entry = entry_res.unwrap();
                let path = entry.into_path();

                let str = path.clone().into_os_string().into_string().unwrap();
                // File paths must not contain any excluded strings.
                // The file must not be a `build.rs` file.
                // The file must be a `.rs` file.
                let a = !exclude.iter().any(|e| str.contains(e));
                let b = !path.ends_with("build.rs");
                let c = path.extension().map_or(false, |ext| ext == "rs");
                if a && b && c {
                    let path_clone = path.clone();
                    let file = OpenOptions::new().read(true).open(path).unwrap();
                    apply(&args.action, file, |_| {
                        OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(&path_clone)
                            .unwrap()
                    })?;
                }
            }
            Ok(())
        }
        Input::Text(TextArgs { text }) => {
            apply(&args.action, text.as_bytes(), |_| std::io::stdout())
        }
    }
}

fn apply<R: Read, W: Write>(
    action: &Action,
    mut source: R,
    target: impl Fn(R) -> W,
) -> Result<(), Error> {
    let mut buf = Vec::new();
    source.read_to_end(&mut buf).unwrap();
    let str = std::str::from_utf8(&buf).unwrap();

    let ast = syn::parse_file(str).unwrap();

    match action {
        Action::Strip => {
            let mut visitor = StripVisitor(
                str.split('\n')
                    .enumerate()
                    .map(|(i, x)| (i, String::from(x)))
                    .collect(),
            );
            visitor.visit_file(&ast);
            target(source)
                .write_all(String::from(visitor).as_bytes())
                .unwrap();
            Ok(())
        }
        Action::Check => {
            let mut visitor = CheckVisitor(false);
            visitor.visit_file(&ast);
            if visitor.0 {
                Err(Error::Check)
            } else {
                Ok(())
            }
        }
        Action::Fix => {
            let mut visitor = FixVisitor(SegmentedList {
                first: String::new(),
                inner: str
                    .split('\n')
                    .map(|x| (String::from(x), String::new()))
                    .collect(),
            });
            visitor.visit_file(&ast);
            target(source)
                .write_all(String::from(visitor).as_bytes())
                .unwrap();
            Ok(())
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

// A function is considered instruments if it has the `#[instrument]` attribute or the `#[test]` attribute.
fn is_instrumented(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        match &attr.meta {
            syn::Meta::List(syn::MetaList { path, .. }) => matches!(path.segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument"),
            syn::Meta::Path(syn::Path { segments, .. }) => matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "test" || ident == "instrument"),
            syn::Meta::NameValue(_) => false,
        }
    })
}
