use clap::Parser;
use quote::ToTokens;
use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;
use walkdir::WalkDir;
#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    path: Option<PathBuf>,
    #[arg(long)]
    check: bool,
}
struct Visitor(bool);
impl syn::visit_mut::VisitMut for Visitor {
    fn visit_item_fn_mut(&mut self, i: &mut syn::ItemFn) {
        let instrumented = i.attrs.iter().any(|attr| match &attr.meta {
            syn::Meta::Path(syn::Path {
                leading_colon: _,
                segments,
            }) => {
                if let Some(path) = segments.last() {
                    path.ident == "instrument"
                } else {
                    false
                }
            }
            _ => false,
        });
        if !instrumented {
            self.0 = true;
            i.attrs
                .push(syn::parse_quote! { # [tracing :: instrument] });
        }
    }
}
impl syn::visit::Visit<'_> for Visitor {
    fn visit_item_fn(&mut self, i: &syn::ItemFn) {
        let instrumented = i.attrs.iter().any(|attr| match &attr.meta {
            syn::Meta::Path(syn::Path {
                leading_colon: _,
                segments,
            }) => {
                if let Some(path) = segments.last() {
                    path.ident == "instrument"
                } else {
                    false
                }
            }
            _ => false,
        });
        if !instrumented {
            self.0 = true;
        }
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
    let mut visitor = Visitor(false);
    for entry in WalkDir::new(args.path.unwrap_or(PathBuf::from(".")))
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let name = entry.file_name().to_string_lossy();
        if name.ends_with(".rs") {
            let path = entry.into_path();
            let content = std::fs::read_to_string(&path).unwrap();
            let mut ast = syn::parse_file(&content).unwrap();
            if args.check {
                visitor.visit_file(&ast);
            } else {
                visitor.visit_file_mut(&mut ast);
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&path)
                    .unwrap();
                file.write_all(ast.into_token_stream().to_string().as_bytes())
                    .unwrap();
                std::process::Command::new("rustfmt")
                    .arg(path)
                    .output()
                    .unwrap();
            }
        }
    }
    if visitor.0 {
        Err(MissingInstrument)
    } else {
        Ok(())
    }
}
