//! Exports a `skip` attribute macro to allow skipping specific functions with `clippy-tracing`.

/// Labels a given function to be skipped by `clippy-tracing`.
#[proc_macro_attribute]
pub fn clippy_tracing_skip(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    item
}
