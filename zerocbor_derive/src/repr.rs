use core::cmp::Ordering;

use proc_macro2::Ident;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, DeriveInput, Error, Fields, Result, Token, Type};

/// How a struct or enum is laid out on the wire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repr {
    /// Positional: a struct is an array, a fieldless enum is a bare index.
    Array,
    /// Self-describing: every field is a text-string key followed by its value.
    Map,
}

/// The key a field or variant is written under.
///
/// `array` needs an index and `map` a name, so a `key` is carried as written
/// and interpreted per representation. Only the two forms are matched, so
/// equality is never needed.
#[derive(Clone)]
pub enum Key {
    /// A positional index, for `array`.
    Index(usize),
    /// A text-string key, for `map`.
    Name(String),
}

/// What to do when a required key is missing from a `map`.
///
/// `syn::Path` has no `PartialEq`; the configuration is built once and only read.
#[derive(Clone)]
pub enum Default {
    /// No `default` attribute: a missing key is an error.
    None,
    /// `#[cbor(default)]`: fill from `Default::default()`.
    FromTrait,
    /// `#[cbor(default = "path")]`: fill from the named function.
    FromPath(syn::Path),
}

/// The container-level configuration from `#[cbor(...)]`.
pub struct ContainerConfig {
    /// How the value is laid out.
    pub repr: Repr,
    /// `#[cbor(tag = N)]`: the tag number this value is written under.
    pub tag: Option<u64>,
    /// `#[cbor(c_enum)]`: write a fieldless enum as its discriminant.
    ///
    /// Already the default, so the attribute only documents the intent.
    pub c_enum: bool,
    /// `#[cbor(allow_unknown_fields)]`: skip keys the struct does not name.
    ///
    /// Off by default, so a newer writer's input is reported, not half-read.
    pub allow_unknown_fields: bool,
}

/// The field-level configuration from `#[cbor(...)]`.
pub struct FieldConfig {
    /// An explicit `key`, if one was given.
    pub key: Option<Key>,
    /// `#[cbor(ignore)]`: leave the field off the wire entirely.
    pub ignore: bool,
    /// `as_bytes`, defaulting to `true` for the three byte-slice shapes.
    pub as_bytes: Option<bool>,
    /// What to do about a missing key.
    pub default: Default,
}

/// A parsed `#[cbor(...)]` argument.
enum CborArg {
    /// A bare word, e.g. `map` or `ignore`.
    Flag(Ident),
    /// `name = true`, e.g. `as_bytes = true`.
    Bool(Ident, bool),
    /// `name = "text"`, e.g. `key = "name"`.
    Text(Ident, String),
    /// `name = 3`, e.g. `key = 3`.
    Int(Ident, usize),
}

/// The parsed arguments of one container or field.
struct AttrArgs {
    args: Vec<CborArg>,
}

impl AttrArgs {
    /// Parses every `#[cbor(...)]` attribute on `attrs`.
    fn collect(attrs: &[Attribute]) -> Result<Self> {
        let mut args = Vec::new();
        for attr in attrs {
            if !attr.path().is_ident("cbor") {
                continue;
            }
            // A bare `#[cbor]` is a harmless no-op, not a build failure.
            if matches!(attr.meta, syn::Meta::Path(_)) {
                continue;
            }
            let parsed = attr
                .parse_args_with(Punctuated::<CborArg, Token![,]>::parse_terminated)
                .map_err(|err| {
                    Error::new_spanned(
                        attr,
                        format!(
                            "malformed `cbor` attribute: {err}; expected something like \
                             `#[cbor(map)]`, `#[cbor(key = 0)]`, `#[cbor(ignore)]` or \
                             `#[cbor(as_bytes = false)]`"
                        ),
                    )
                })?;
            args.extend(parsed);
        }
        Ok(AttrArgs { args })
    }

    /// Every bare flag with the given name.
    fn flags(&self, name: &str) -> impl Iterator<Item = &Ident> {
        self.args.iter().filter_map(move |arg| match arg {
            CborArg::Flag(flag) if flag == name => Some(flag),
            _ => None,
        })
    }

    /// The first `name = <text>` value, if present.
    fn text(&self, name: &str) -> Option<String> {
        self.args.iter().find_map(|arg| match arg {
            CborArg::Text(key, value) if key == name => Some(value.clone()),
            _ => None,
        })
    }

    /// The first `name = <int>` value, if present.
    fn int(&self, name: &str) -> Option<usize> {
        self.args.iter().find_map(|arg| match arg {
            CborArg::Int(key, value) if key == name => Some(*value),
            _ => None,
        })
    }

    /// The first `name = <bool>` value, if present.
    fn bool(&self, name: &str) -> Option<bool> {
        self.args.iter().find_map(|arg| match arg {
            CborArg::Bool(key, value) if key == name => Some(*value),
            _ => None,
        })
    }

    /// Whether a bare flag was given, reporting a duplicate as an error.
    fn flag(&self, name: &str, span: proc_macro2::Span) -> Result<bool> {
        let mut found = false;
        for _ in self.flags(name) {
            if found {
                return Err(Error::new(span, format!("duplicate `{name}` attribute")));
            }
            found = true;
        }
        Ok(found)
    }

    /// Rejects any argument whose name is not in `allowed`, so a typo fails the
    /// build rather than silently doing nothing.
    fn reject_unknown(&self, allowed: &[&str]) -> Result<()> {
        for arg in &self.args {
            let name = match arg {
                CborArg::Flag(flag) => flag,
                CborArg::Bool(flag, _) | CborArg::Text(flag, _) | CborArg::Int(flag, _) => flag,
            };
            if !allowed.iter().any(|ok| name == ok) {
                return Err(Error::new(
                    name.span(),
                    format!(
                        "unknown attribute `{name}`; expected one of {}",
                        allowed
                            .iter()
                            .map(|ok| format!("`{ok}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ));
            }
        }
        Ok(())
    }
}

impl syn::parse::Parse for CborArg {
    fn parse(input: syn::parse::ParseStream<'_>) -> Result<Self> {
        let name: Ident = input.parse()?;
        if !input.peek(Token![=]) {
            return Ok(CborArg::Flag(name));
        }
        input.parse::<Token![=]>()?;
        // The literal's kind is the shape, so `key = 0` and `key = "name"`
        // both parse and neither is a type error.
        if input.peek(syn::LitStr) {
            let value: syn::LitStr = input.parse()?;
            return Ok(CborArg::Text(name, value.value()));
        }
        if input.peek(syn::LitBool) {
            let value: syn::LitBool = input.parse()?;
            return Ok(CborArg::Bool(name, value.value()));
        }
        if input.peek(syn::LitInt) {
            let value: syn::LitInt = input.parse()?;
            let parsed = value.base10_parse::<usize>()?;
            return Ok(CborArg::Int(name, parsed));
        }
        Err(input.error("expected a string, integer, or boolean literal"))
    }
}

impl ContainerConfig {
    /// Reads the container's `#[cbor(...)]` attributes.
    pub fn from_attrs(input: &DeriveInput) -> Result<Self> {
        let args = AttrArgs::collect(&input.attrs)?;
        args.reject_unknown(&["array", "map", "c_enum", "allow_unknown_fields", "tag"])?;

        // `map` wins if both are given: the more explicit one adds information.
        let repr = if args.flags("map").next().is_some() {
            Repr::Map
        } else {
            Repr::Array
        };
        let tag = match (args.flags("tag").next(), args.int("tag")) {
            (Some(_), _) => {
                return Err(Error::new_spanned(
                    &input.ident,
                    "`tag` needs a number, as in `#[cbor(tag = 55799)]`",
                ));
            }
            (None, Some(number)) => Some(number as u64),
            (None, None) => None,
        };
        let c_enum = args.flag("c_enum", input.ident.span())?;
        if c_enum {
            match &input.data {
                syn::Data::Enum(data)
                    if data
                        .variants
                        .iter()
                        .all(|variant| matches!(variant.fields, syn::Fields::Unit)) => {}
                _ => {
                    return Err(Error::new_spanned(
                        &input.ident,
                        "`c_enum` requires a fieldless enum",
                    ));
                }
            }
            if matches!(repr, Repr::Map) {
                return Err(Error::new_spanned(
                    &input.ident,
                    "`c_enum` encodes an integer and cannot be combined with `map`",
                ));
            }
            for variant in match &input.data {
                syn::Data::Enum(data) => &data.variants,
                _ => unreachable!(),
            } {
                if variant_tag(variant, None)?.is_some() {
                    return Err(Error::new_spanned(
                        &variant.ident,
                        "`c_enum` does not support per-variant tags; tag the whole enum instead",
                    ));
                }
            }
        }
        Ok(ContainerConfig {
            repr,
            tag,
            c_enum,
            allow_unknown_fields: args.flag("allow_unknown_fields", input.ident.span())?,
        })
    }
}

impl FieldConfig {
    /// Reads one field's `#[cbor(...)]` attributes.
    pub fn from_syn(field: &syn::Field) -> Result<Self> {
        let args = AttrArgs::collect(&field.attrs)?;
        args.reject_unknown(&["key", "ignore", "as_bytes", "default"])?;

        // An index or a name, depending on the literal; which is correct is
        // decided per representation later.
        let key = match (args.int("key"), args.text("key")) {
            (Some(_), Some(_)) => {
                return Err(Error::new_spanned(&field.ty, "duplicate `key` attribute"));
            }
            (Some(index), None) => Some(Key::Index(index)),
            (None, Some(name)) => Some(Key::Name(name)),
            (None, None) => None,
        };

        let default = match (args.flags("default").next(), args.text("default")) {
            (Some(_), Some(_)) => {
                return Err(Error::new_spanned(
                    &field.ty,
                    "`default` cannot be both a flag and a path",
                ));
            }
            (Some(_), None) => Default::FromTrait,
            (None, Some(path)) => {
                let parsed: syn::Path = syn::parse_str(&path).map_err(|err| {
                    Error::new(
                        proc_macro2::Span::call_site(),
                        format!("`default = \"{path}\"` is not a path: {err}"),
                    )
                })?;
                Default::FromPath(parsed)
            }
            (None, None) => Default::None,
        };

        let as_bytes = args.bool("as_bytes");
        if as_bytes.is_some() && bytes_read(&field.ty).is_none() {
            return Err(Error::new_spanned(
                &field.ty,
                "`as_bytes` applies to `&[u8]`, `Cow<'a, [u8]>` and `Vec<u8>` \
                 fields, which are the types the blanket slice and `Vec` impls \
                 would otherwise encode as arrays",
            ));
        }

        Ok(FieldConfig {
            key,
            ignore: args.flag(
                "ignore",
                field
                    .ident
                    .as_ref()
                    .map_or_else(proc_macro2::Span::call_site, |ident| ident.span()),
            )?,
            as_bytes,
            default,
        })
    }
}

/// A field that takes part in the representation.
pub struct ActiveField<'a> {
    /// The field's own name, if the struct is named.
    pub ident: Option<&'a Ident>,
    /// The field's type.
    pub ty: &'a Type,
    /// The field's configuration.
    pub config: FieldConfig,
}

impl ActiveField<'_> {
    /// The expression that reads this field out of `self`.
    pub fn accessor(&self, index: usize) -> proc_macro2::TokenStream {
        match self.ident {
            Some(ident) => quote! { self.#ident },
            None => {
                let index = syn::Index::from(index);
                quote! { self.#index }
            }
        }
    }

    /// The key this field is written under in map mode: an explicit
    /// `key = "name"`, else the field's own name.
    pub fn map_key(&self) -> Result<String> {
        match &self.config.key {
            Some(Key::Name(name)) => Ok(name.clone()),
            // An index here would put a number where a string belongs.
            Some(Key::Index(index)) => Err(Error::new_spanned(
                self.ty,
                format!(
                    "`key = {index}` is a positional index, but map mode needs a \
                         name; write `key = \"name\"`"
                ),
            )),
            None => Ok(self.ident.expect("named field has an ident").to_string()),
        }
    }

    /// The position this field occupies in array mode, if one was given.
    pub fn array_index(&self) -> Result<Option<usize>> {
        match &self.config.key {
            Some(Key::Index(index)) => Ok(Some(*index)),
            Some(Key::Name(name)) => Err(Error::new_spanned(
                self.ty,
                format!(
                    "`key = \"{name}\"` is a map key, but array mode needs an \
                         index; write `key = 0`"
                ),
            )),
            None => Ok(None),
        }
    }

    /// Whether the field is written as a byte string. Defaults to true for a
    /// byte-slice shape, as in the sibling crate.
    pub fn is_bytes(&self) -> bool {
        self.config
            .as_bytes
            .unwrap_or_else(|| bytes_read(self.ty).is_some())
    }

    /// How this field is read back when it is a byte string.
    pub fn bytes_shape(&self) -> Option<BytesRead> {
        self.is_bytes().then(|| bytes_read(self.ty)).flatten()
    }
}

/// How a byte-string field is read back, which depends on whether it can borrow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BytesRead {
    /// `&'de [u8]`, borrowed straight out of the input.
    BorrowedRef,
    /// `Cow<'a, [u8]>`, borrowed when the reader can lend, owned otherwise.
    Cow,
    /// `Vec<u8>`, always copied out of the reader.
    Owned,
}

/// Whether `ty` is a `&[u8]`, `Cow<[u8]>`, or `Vec<u8>`.
pub fn bytes_read(ty: &Type) -> Option<BytesRead> {
    match ty {
        Type::Reference(reference) if is_u8_slice(&reference.elem) => Some(BytesRead::BorrowedRef),
        Type::Path(path) if path.qself.is_none() => {
            if is_cow_u8_slice(&path.path) {
                Some(BytesRead::Cow)
            } else if is_vec_u8_path(&path.path) {
                Some(BytesRead::Owned)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Whether `path` names `Cow<'_, [u8]>`.
fn is_cow_u8_slice(path: &syn::Path) -> bool {
    let Some(segment) = path.segments.last() else {
        return false;
    };
    if segment.ident != "Cow" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return false;
    };
    args.args.iter().any(|arg| match arg {
        syn::GenericArgument::Type(Type::Slice(slice)) => is_u8(&slice.elem),
        _ => false,
    })
}

/// Whether `path` names `Vec<u8>`.
fn is_vec_u8_path(path: &syn::Path) -> bool {
    let Some(segment) = path.segments.last() else {
        return false;
    };
    if segment.ident != "Vec" {
        return false;
    }
    matches!(&segment.arguments, syn::PathArguments::AngleBracketed(args)
        if args.args.iter().any(|arg| matches!(arg, syn::GenericArgument::Type(inner) if is_u8(inner))))
}

/// Whether `ty` is `[u8]`, ignoring any lifetime.
fn is_u8_slice(ty: &Type) -> bool {
    matches!(ty, Type::Slice(slice) if is_u8(&slice.elem))
}

/// Whether `ty` is the `u8` primitive.
fn is_u8(ty: &Type) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none() && path.path.is_ident("u8"))
}

/// Collects the fields of `fields` that are not `#[cbor(ignore)]`d.
pub fn active_fields(fields: &syn::FieldsNamed) -> Result<Vec<ActiveField<'_>>> {
    let mut out = Vec::with_capacity(fields.named.len());
    for field in &fields.named {
        let config = FieldConfig::from_syn(field)?;
        if config.ignore {
            continue;
        }
        out.push(ActiveField {
            ident: field.ident.as_ref(),
            ty: &field.ty,
            config,
        });
    }
    Ok(out)
}

/// The names of the fields that `#[cbor(ignore)]` removes from the wire.
///
/// A struct literal initializes every field, so decoding names them; the value
/// comes from `Default`.
pub fn ignored_idents(fields: &syn::Fields) -> Result<Vec<Ident>> {
    let named = match fields {
        Fields::Named(named) => named,
        _ => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for field in &named.named {
        if FieldConfig::from_syn(field)?.ignore {
            out.push(field.ident.clone().expect("named field has an ident"));
        }
    }
    Ok(out)
}

/// The name a variant is written under in map mode: an explicit
/// `#[cbor(key = "...")]`, else the variant's own name. A numeric key is
/// rejected here rather than later.
pub fn variant_name(variant: &syn::Variant) -> Result<String> {
    let args = AttrArgs::collect(&variant.attrs)?;
    args.reject_unknown(&["key", "ignore", "tag"])?;
    match (args.int("key"), args.text("key")) {
        (Some(_), Some(_)) => Err(Error::new_spanned(
            &variant.ident,
            "duplicate `key` attribute",
        )),
        (Some(index), None) => Err(Error::new_spanned(
            &variant.ident,
            format!(
                "`key = {index}` is a positional index, but map mode needs a name; \
                 write `key = \"Name\"`"
            ),
        )),
        (None, Some(name)) => Ok(name),
        (None, None) => Ok(variant.ident.to_string()),
    }
}

/// A field as declared, ignoring whether it is on the wire.
///
/// The decoder needs the whole declaration to build the struct literal; the
/// writer only needs the active subset.
pub struct DeclaredField<'a> {
    /// The field's own name, if the struct or variant is named.
    pub ident: Option<&'a Ident>,
    /// The field's type.
    pub ty: &'a Type,
}

/// Every field of a struct or variant, in declaration order.
pub fn all_fields(fields: &Fields) -> Result<Vec<DeclaredField<'_>>> {
    Ok(match fields {
        Fields::Named(named) => named
            .named
            .iter()
            .map(|field| DeclaredField {
                ident: field.ident.as_ref(),
                ty: &field.ty,
            })
            .collect(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .map(|field| DeclaredField {
                ident: None,
                ty: &field.ty,
            })
            .collect(),
        Fields::Unit => Vec::new(),
    })
}

/// The tag number a variant is written under.
///
/// A variant naming none inherits the enum's.
pub fn variant_tag(variant: &syn::Variant, inherited: Option<u64>) -> Result<Option<u64>> {
    let args = AttrArgs::collect(&variant.attrs)?;
    if args.flags("tag").next().is_some() {
        return Err(Error::new_spanned(
            &variant.ident,
            "`tag` needs a number, as in `#[cbor(tag = 55799)]`",
        ));
    }
    match args.int("tag") {
        Some(number) => Ok(Some(number as u64)),
        None => Ok(inherited),
    }
}

/// Reads a newtype struct's single field.
///
/// A tuple struct of more than one field is a sequence, which is the array
/// representation, and a unit struct has no field to delegate to.
pub fn newtype_field(data: &syn::DataStruct) -> Result<&syn::Field> {
    let span = data.struct_token.span;
    match &data.fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => Ok(&fields.unnamed[0]),
        Fields::Unnamed(fields) => Err(Error::new(
            span,
            format!(
                "a tuple struct with {} fields has no single value to encode; \
                 give the fields names, or write the array form by hand",
                fields.unnamed.len()
            ),
        )),
        Fields::Unit => Err(Error::new(
            span,
            "a unit struct has no value to encode; a fieldless enum is the \
             shape for a value that is just a discriminant",
        )),
        Fields::Named(_) => Err(Error::new(span, "expected a newtype struct")),
    }
}

/// The single field of a newtype struct, configured the way any other is.
///
/// A newtype's field is unnamed, so the container code takes its configuration
/// from here rather than from the shape. `as_bytes` in particular has to keep
/// working: a bignum newtype is a byte string, and without it the bytes would
/// be written as an array of integers.
pub fn newtype_active_field(field: &syn::Field) -> Result<ActiveField<'_>> {
    let config = FieldConfig::from_syn(field)?;
    if let Some(Key::Index(_)) = config.key {
        return Err(Error::new_spanned(
            &field.ty,
            "`key` does not apply to a newtype's only field, which is the value \
             itself rather than one entry of a container",
        ));
    }
    Ok(ActiveField {
        ident: None,
        ty: &field.ty,
        config,
    })
}

/// The canonical order of two map keys, per RFC 8949 Section 4.2.1.
///
/// The order is bytewise over the keys' own encodings, and a text string's
/// initial byte rises with its length — `0x60` through `0x77` for the inline
/// form, then one byte per wider length — so that order is length first and then
/// the bytes. Sorting at expansion time is what makes it free at run time.
pub fn canonical_key_order(a: &str, b: &str) -> Ordering {
    a.len()
        .cmp(&b.len())
        .then_with(|| a.as_bytes().cmp(b.as_bytes()))
}
