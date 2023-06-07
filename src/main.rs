use clap::Parser;

use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

// cargo run --release -- --path /home/ec2-user/firecracker --exclude 

// TODO When adding on `fmt` for `Display` always add `skip(f)`.
// TODO When adding on functions which return a mutable reference do not add `ret`.
#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    path: Option<PathBuf>,
    // Check if all functions are instrumented but don't make any change.
    #[arg(long)]
    check: bool,
    #[arg(long)]
    exclude: Vec<String>
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
            "{}{}",
            list.first,
            list.inner
                .into_iter()
                .map(|(x, y)| format!("{x}\n{y}"))
                .collect::<String>()
        )
    }
}

struct Visitor {
    text: SegmentedList,
    changed: bool,
    check: bool,
}
impl Visitor {
    fn new(text: String, check: bool) -> Self {
        Self {
            text: SegmentedList {
                first: String::new(),
                inner: text
                    .split('\n')
                    .map(|x| (String::from(x), String::new()))
                    .collect(),
            },
            changed: false,
            check,
        }
    }
}
impl syn::visit::Visit<'_> for Visitor {
    fn visit_impl_item_fn(&mut self, i: &syn::ImplItemFn) {
        let mut instrumented = false;
        let mut test = false;
        for attr in i.attrs.iter() {
            match &attr.meta {
                syn::Meta::Path(syn::Path { segments, .. }) => {
                    if let Some(path) = segments.last() {
                        if path.ident == "instrument" {
                            instrumented = true;
                            break;
                        }
                    }
                }
                syn::Meta::List(syn::MetaList { path, .. }) => {
                    if let Some(path) = path.segments.last() {
                        if path.ident == "instrument" {
                            instrumented = true;
                            break;
                        }
                    }
                }
                _ => {}
            };

            if let syn::Meta::Path(syn::Path { segments, .. }) = &attr.meta {
                if let Some(path) = segments.last() {
                    if path.ident == "test" {
                        test = true;
                        break;
                    }
                }
            }
        }

        // If the function is not instrument, is not a test, and is not const.
        if !instrumented && !test && i.sig.constness.is_none() {
            self.changed = true;
            if !self.check {
                let line = i.sig.span().start().line;
                self.text
                    .insert_before(line - 1, "#[tracing::instrument(level = \"trace\", ret)]\n");
            }
        }
        self.visit_block(&i.block);
    }
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        let mut instrumented = false;
        let mut test = false;
        for attr in i.attrs.iter() {
            match &attr.meta {
                syn::Meta::Path(syn::Path { segments, .. }) => {
                    if let Some(path) = segments.last() {
                        if path.ident == "instrument" {
                            instrumented = true;
                            break;
                        }
                    }
                }
                syn::Meta::List(syn::MetaList { path, .. }) => {
                    if let Some(path) = path.segments.last() {
                        if path.ident == "instrument" {
                            instrumented = true;
                            break;
                        }
                    }
                }
                _ => {}
            };

            if let syn::Meta::Path(syn::Path { segments, .. }) = &attr.meta {
                if let Some(path) = segments.last() {
                    if path.ident == "test" {
                        test = true;
                        break;
                    }
                }
            }
        }

        // If the function is not instrument, is not a test, and is not const.
        if !instrumented && !test && i.sig.constness.is_none() {
            self.changed = true;
            if !self.check {
                let line = i.sig.span().start().line;
                self.text
                    .insert_before(line - 1, "#[tracing::instrument(level = \"trace\", ret)]\n");
            }
        }

        self.visit_block(&i.block);
    }
}
#[derive(Debug)]
struct MissingInstrument;
impl fmt::Display for MissingInstrument {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Found a function not annotated with `#[tracing::instrument]`"
        )
    }
}
impl std::error::Error for MissingInstrument {}
fn main() -> Result<(), MissingInstrument> {
    let args = Args::parse();

    let mut changed = false;
    for entry in WalkDir::new(args.path.unwrap_or(PathBuf::from(".")))
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let name = entry.file_name().to_string_lossy();

        // Skip file paths containing excluded strings.
        for exclude in args.exclude {
            if exclude.contains(name);
            continue;
        }

        // Skip build.rs files.
        if name.ends_with("build.rs") {
            continue;
        }
        if name.ends_with(".rs") {
            let path = entry.into_path();
            let content = std::fs::read_to_string(&path).unwrap();
            let ast = syn::parse_file(&content).unwrap();
            let mut visitor = Visitor::new(content, args.check);
            visitor.visit_file(&ast);
            if !args.check {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&path)
                    .unwrap();
                file.write_all(String::from(visitor.text).as_bytes())
                    .unwrap();
            }
            if visitor.changed {
                changed = true;
                if args.check {
                    break;
                }
            }
        }
    }
    if changed {
        Err(MissingInstrument)
    } else {
        Ok(())
    }
}
