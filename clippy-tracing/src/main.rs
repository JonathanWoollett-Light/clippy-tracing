#![warn(clippy::pedantic)]

use clap::{Args, Parser, Subcommand, ValueEnum};

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use std::fs::OpenOptions;

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
    #[arg(long, value_delimiter = ',')]
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
            let line = i.sig.span().start().line;
            self.0.insert_before(line - 1, INSTRUMENT);
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        let attr = check_attributes(&i.attrs);
        if !attr.instrumented && !attr.skipped && !attr.test && i.sig.constness.is_none() {
            let line = i.sig.span().start().line;
            self.0.insert_before(line - 1, INSTRUMENT);
        }
        self.visit_block(&i.block);
    }
}

const INSTRUMENT: &str = "#[tracing::instrument(level = \"trace\", ret)]";

fn main() -> Result<(), ApplyError> {
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

                let path_str = path.clone().into_os_string().into_string().unwrap();
                // File paths must not contain any excluded strings.
                // The file must not be a `build.rs` file.
                // The file must be a `.rs` file.
                let a = !exclude.iter().any(|e| path_str.contains(e));
                let b = !path.ends_with("build.rs");
                let c = path.extension().map_or(false, |ext| ext == "rs");
                if a && b && c {
                    let path_clone = path.clone();
                    let file = OpenOptions::new().read(true).open(path).unwrap();
                    let Ok(res) = apply(&args.action, file, |_| {
                        OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(&path_clone)
                            .unwrap()
                    }) else { panic!("Failed to pass file {path_str}") };

                    if let Some(span) = res {
                        eprintln!(
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
        Input::Text(TextArgs { text }) => {
            let res = apply(&args.action, text.as_bytes(), |_| std::io::stdout())?;
            if let Some(span) = res {
                eprintln!(
                    "Missing instrumentation at {}:{}.",
                    span.start().line,
                    span.start().column
                );
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

#[derive(Debug)]
pub enum ApplyError {
    Parse(syn::parse::Error),
}

fn apply<R: Read, W: Write>(
    action: &Action,
    mut source: R,
    target: impl Fn(R) -> W,
) -> Result<Option<proc_macro2::Span>, ApplyError> {
    let mut buf = Vec::new();
    source.read_to_end(&mut buf).unwrap();
    let text = std::str::from_utf8(&buf).unwrap();

    let ast = syn::parse_file(text).map_err(ApplyError::Parse)?;

    match action {
        Action::Strip => {
            let mut visitor = StripVisitor(
                text.split('\n')
                    .enumerate()
                    .map(|(i, x)| (i, String::from(x)))
                    .collect(),
            );
            visitor.visit_file(&ast);
            target(source)
                .write_all(String::from(visitor).as_bytes())
                .unwrap();
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
            target(source)
                .write_all(String::from(visitor).as_bytes())
                .unwrap();
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
            syn::Meta::Path(syn::Path { segments, .. }) => {
                matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "test" || ident == "proof")
            }
            syn::Meta::Path(syn::Path { segments, .. }) => {
                matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "proof")
            }
            _ => false,
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
