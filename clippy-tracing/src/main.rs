//! A tool to add, remove and check for `tracing::instrument` in large projects where it is infeasible to manually add it to thousands of functions.

#![warn(clippy::pedantic, clippy::restriction)]
#![allow(
    clippy::blanket_clippy_restriction_lints,
    clippy::single_call_fn,
    clippy::absolute_paths,
    clippy::pattern_type_mismatch,
    clippy::implicit_return,
    clippy::question_mark_used,
    clippy::missing_trait_methods,
    clippy::min_ident_chars,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::wildcard_enum_match_arm,
    clippy::arithmetic_side_effects
)]

extern crate alloc;

use alloc::fmt;
use clap::{Parser, ValueEnum};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use std::error::Error;

/// The command line arguments for the application.
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

/// The action to take.
#[derive(Clone, ValueEnum)]
enum Action {
    /// Checks `tracing::instrument` is on all functions.
    Check,
    /// Adds `tracing::instrument` to all functions.
    Fix,
    /// Removes `tracing::instrument` from all functions.
    Strip,
}

/// A list of text lines split so that newlines can be efficiently inserted between them.
struct SegmentedList {
    /// The first new line.
    first: String,
    /// The inner vector used to contain the original lines `.0` and the new lines `.1`.
    inner: Vec<(String, String)>,
}
impl SegmentedList {
    /// Sets the text line before `line` to `text`.
    fn set_before(&mut self, line: usize, text: String) -> bool {
        let s = if let Some(i) = line.checked_sub(1) {
            let Some(mut_ref) = self.inner.get_mut(i) else {
                return false;
            };
            &mut mut_ref.1
        } else {
            &mut self.first
        };
        *s = text;
        true
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

/// Visitor for the `strip` action.
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
            let start = instrument.span().start().line - 1;
            let end = instrument.span().end().line;
            for line in start..end {
                self.0.remove(&line);
            }
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        if let Some(instrument) = find_instrumented(&i.attrs) {
            let start = instrument.span().start().line - 1;
            let end = instrument.span().end().line;
            for line in start..end {
                self.0.remove(&line);
            }
        }
        self.visit_block(&i.block);
    }
}

/// Visitor for the `check` action.
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

/// Visitor for the `fix` action.
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
            let indent_attr = format!("{}{attr_string}", " ".repeat(indent));
            self.0.set_before(line - 1, indent_attr);
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        let attr = check_attributes(&i.attrs);

        if !attr.instrumented && !attr.skipped && !attr.test && i.sig.constness.is_none() {
            let line = i.span().start().line;

            let attr_string = instrument(&i.sig);
            let indent = i.span().start().column;
            let indent_attr = format!("{}{attr_string}", " ".repeat(indent));
            self.0.set_before(line - 1, indent_attr);
        }
        self.visit_block(&i.block);
    }
}

/// Returns the instrument macro for a given function signature.
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
    let args = itertools::intersperse(iter, String::from(", ")).collect::<String>();

    format!("#[tracing::instrument(level = \"trace\", skip({args}))]")
}

use std::process::ExitCode;

/// Type to return from `main` to support returning an error then handling it.
#[repr(u8)]
enum Exit {
    /// Process completed successfully.
    Ok = 0,
    /// Process encountered an error.
    Error = 1,
    /// Process ran `check` action and found missing instrumentation.
    Check = 2,
}
#[allow(clippy::as_conversions)]
impl std::process::Termination for Exit {
    fn report(self) -> ExitCode {
        ExitCode::from(self as u8)
    }
}

fn main() -> Exit {
    match exec() {
        Err(err) => {
            eprintln!("Error: {err}");
            Exit::Error
        }
        Ok(None) => Exit::Ok,
        Ok(Some((path, line, column))) => {
            println!(
                "Missing instrumentation at {}:{line}:{column}.",
                path.display()
            );
            Exit::Check
        }
    }
}

/// Error for [`exec`].
#[derive(Debug)]
enum ExecError {
    /// Failed to read entry in file path.
    Entry(walkdir::Error),
    /// Failed to parse file path to string.
    String,
    /// Failed to open file.
    File(std::io::Error),
    /// Failed to run apply function.
    Apply(ApplyError),
}
impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry(entry) => write!(f, "Failed to read entry in file path: {entry}"),
            Self::String => write!(f, "Failed to parse file path to string."),
            Self::File(file) => write!(f, "Failed to open file: {file}"),
            Self::Apply(apply) => write!(f, "Failed to run apply function: {apply}"),
        }
    }
}

impl Error for ExecError {}

/// Wraps functionality from `main` to support returning an error then handling it.
fn exec() -> Result<Option<(PathBuf, usize, usize)>, ExecError> {
    let args = CommandLineArgs::parse();

    let path = args.path.unwrap_or(PathBuf::from("."));
    for entry_res in WalkDir::new(path).follow_links(true) {
        let entry = entry_res.map_err(ExecError::Entry)?;
        let entry_path = entry.into_path();

        let path_str = entry_path.to_str().ok_or(ExecError::String)?;
        // File paths must not contain any excluded strings.
        let a = !args.exclude.iter().any(|e| path_str.contains(e));
        // The file must not be a `build.rs` file.
        let b = !entry_path.ends_with("build.rs");
        // The file must be a `.rs` file.
        let c = entry_path.extension().map_or(false, |ext| ext == "rs");

        if a && b && c {
            let file = OpenOptions::new()
                .read(true)
                .open(&entry_path)
                .map_err(ExecError::File)?;
            let res = apply(&args.action, file, |_| {
                OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&entry_path)
            })
            .map_err(ExecError::Apply)?;

            if let Some(span) = res {
                return Ok(Some((entry_path, span.start().line, span.start().column)));
            }
        }
    }
    Ok(None)
}

/// Error for [`apply`].
#[derive(Debug)]
enum ApplyError {
    /// Failed to read file.
    Read(std::io::Error),
    /// Failed to parse file to utf8.
    Utf(core::str::Utf8Error),
    /// Failed to parse file to syn ast.
    Syn(syn::parse::Error),
    /// Failed to get write target.
    Target(std::io::Error),
    /// Failed to write result to target.
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

/// Apply the given action to the given source and outputs the result to the target produced by the
/// given closure.
fn apply<R: Read, W: Write>(
    action: &Action,
    mut source: R,
    target: impl Fn(R) -> Result<W, std::io::Error>,
) -> Result<Option<proc_macro2::Span>, ApplyError> {
    let mut buf = Vec::new();
    source.read_to_end(&mut buf).map_err(ApplyError::Read)?;
    let text = core::str::from_utf8(&buf).map_err(ApplyError::Utf)?;

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

/// Finds the `#[instrument]` attribute on a function.
fn find_instrumented(attrs: &[syn::Attribute]) -> Option<&syn::Attribute> {
    attrs.iter().find(|attr| {
        match &attr.meta {
            syn::Meta::List(syn::MetaList { path, .. }) => matches!(path.segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument"),
            syn::Meta::Path(syn::Path { segments, .. }) => matches!(segments.last(), Some(syn::PathSegment { ident, .. }) if ident == "instrument"),
            syn::Meta::NameValue(_) => false,
        }
    })
}

/// The description of attributes on a function signature we care about.
struct Desc {
    /// Does the function have the `#[tracing::instrument]` attribute macro?
    instrumented: bool,
    /// Does the function have the `#[clippy_tracing_attributes::clippy_tracing_skip]` attribute macro?
    skipped: bool,
    /// Does the function have the `#[test]` attribute macro?
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
