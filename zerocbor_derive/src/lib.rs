//! Derive macros for `zerocbor`.
//!
//! `c_enum` requires a fieldless enum and an integer representation:
//!
//! ```compile_fail
//! #[derive(zerocbor_derive::ToCbor)]
//! #[cbor(c_enum)]
//! enum Data { Value(u8) }
//! ```
//!
//! ```compile_fail
//! #[derive(zerocbor_derive::FromCbor)]
//! #[cbor(c_enum, map)]
//! enum Names { First }
//! ```
//!
//! Tags on a `c_enum` apply to the whole enum, not individual variants:
//!
//! ```compile_fail
//! #[derive(zerocbor_derive::FromCbor)]
//! #[cbor(c_enum)]
//! enum Tagged { #[cbor(tag = 42)] First }
//! ```

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod from_cbor;
mod repr;
mod to_cbor;
mod util;

/// Derives `zerocbor::ToCbor`. See the [crate-level docs](crate) for the attributes.
#[proc_macro_derive(ToCbor, attributes(cbor, serde))]
pub fn derive_to_cbor(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match to_cbor::expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derives `zerocbor::FromCbor`. See the [crate-level docs](crate) for the attributes.
#[proc_macro_derive(FromCbor, attributes(cbor, serde))]
pub fn derive_from_cbor(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match from_cbor::expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
