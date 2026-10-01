use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DataEnum, DataStruct, DeriveInput, Fields, Ident, Result};

use crate::repr::{ActiveField, ContainerConfig, Default, Repr};
use crate::util;

/// Expands `FromCbor` for a struct or an enum.
pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    let config = ContainerConfig::from_attrs(input)?;
    let name = &input.ident;
    // The impl header always names `'de`, whether or not the type borrows.
    let generics = util::add_de_lifetime(&util::add_trait_bounds(&input.generics, true));
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    // The target keeps its own generics; `'de` belongs to the impl header.
    let (_, ty_generics, _) = input.generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => struct_body(&config, name, data)?,
        Data::Enum(data) => enum_body(&config, name, data, &input.attrs)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "FromCbor cannot be derived for a union",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics ::zerocbor::FromCbor<'de> for #name #ty_generics #where_clause {
            fn read<__R: ::zerocbor::Read<'de>>(
                __reader: &mut __R,
            ) -> ::zerocbor::Result<Self> {
                __reader.increment_depth()?;
                let __result = (|| { #body })();
                __reader.decrement_depth();
                __result
            }
        }
    })
}

/// Reads one field, honouring `as_bytes`.
///
/// `&[u8]` borrows, `Cow` borrows or owns, `Vec<u8>` copies.
fn read_field(field: &ActiveField<'_>) -> TokenStream {
    if !field.is_bytes() {
        let ty = field.ty;
        return quote! { <#ty as ::zerocbor::FromCbor<'de>>::read(__reader)? };
    }

    match field.bytes_shape() {
        Some(crate::repr::BytesRead::BorrowedRef) => quote! {
            <&'de [u8] as ::zerocbor::FromCbor<'de>>::read(__reader)?
        },
        Some(crate::repr::BytesRead::Cow) => quote! {
            __reader.read_binary()?
        },
        Some(crate::repr::BytesRead::Owned) => quote! {
            ::core::convert::From::from(__reader.read_binary()?.into_owned())
        },
        None => unreachable!("is_bytes implies a known byte-string shape"),
    }
}

/// Reads a struct in the chosen representation.
fn struct_body(config: &ContainerConfig, name: &Ident, data: &DataStruct) -> Result<TokenStream> {
    // A newtype is its inner value, so it is read as that value rather than as
    // a one-element container, and a tag sits directly in front of it.
    if matches!(data.fields, Fields::Unnamed(_)) {
        return newtype_body(config, name, data);
    }
    let fields = util::struct_fields(data)?;
    // An ignored field is not on the wire; it is filled from `Default`.
    let ignored: Vec<Ident> = crate::repr::ignored_idents(&data.fields)?;

    match config.repr {
        Repr::Array => {
            // Reads and `null` gaps are built in one pass, in wire order: a
            // `key = N` is preceded by the positions still unfilled.
            let mut reads: Vec<TokenStream> = Vec::new();
            let mut entries: Vec<TokenStream> = Vec::new();
            let mut written = 0usize;
            for field in &fields {
                let ident = field.ident.expect("named field");
                let ty = field.ty;
                if let Some(target) = field.array_index()? {
                    for _ in written..target {
                        reads.push(quote! { __reader.read_null()?; });
                    }
                    written = target;
                }
                let value = read_field(field);
                reads.push(quote! {
                    let #ident: #ty = #value;
                });
                entries.push(quote! { #ident });
                written += 1;
            }
            for ident in &ignored {
                entries.push(quote! {
                    #ident: ::core::default::Default::default()
                });
            }
            let len = written;

            Ok(with_tag(
                config.tag,
                quote! {
                    let __len = __reader.check_array_len(#len)?;
                    #(#reads)*
                    // A break ends an indefinite-length array, after the fields.
                    __reader.finish_array(__len)?;
                    ::core::result::Result::Ok(#name { #(#entries),* })
                },
            ))
        }
        Repr::Map => {
            let any_default = fields
                .iter()
                .any(|field| !matches!(field.config.default, Default::None));
            Ok(with_tag(
                config.tag,
                map_body(config, name, &fields, &ignored, any_default),
            ))
        }
    }
}

/// Reads a newtype struct as its inner value, under a tag if one was declared.
fn newtype_body(config: &ContainerConfig, name: &Ident, data: &DataStruct) -> Result<TokenStream> {
    let field = crate::repr::newtype_field(data)?;
    let active = crate::repr::newtype_active_field(field)?;
    let ty = active.ty;
    let body = if active.config.ignore {
        quote! {
            __reader.read_null()?;
            ::core::result::Result::Ok(#name(::core::default::Default::default()))
        }
    } else {
        let value = read_field(&active);
        quote! {
            let __inner: #ty = #value;
            ::core::result::Result::Ok(#name(__inner))
        }
    };
    Ok(with_tag(config.tag, body))
}

/// Puts a tag check in front of a read, if the type declared one.
fn with_tag(tag: Option<u64>, body: TokenStream) -> TokenStream {
    match tag {
        None => body,
        Some(tag) => quote! {
            __reader.check_tag(#tag)?;
            #body
        },
    }
}

/// Builds the map-mode decode for a struct.
///
/// | `default` | `allow_unknown_fields` | length | missing key | unknown key |
/// |-----------|-------------------------|--------|-------------|-------------|
/// | no        | no (default)            | exact  | error       | error       |
/// | yes       | no                      | read   | default     | error       |
/// | no        | yes                     | read   | error       | skip        |
/// | yes       | yes                     | read   | default     | skip        |
fn map_body(
    config: &ContainerConfig,
    name: &Ident,
    fields: &[ActiveField<'_>],
    ignored: &[Ident],
    any_default: bool,
) -> TokenStream {
    let count = fields.len();
    let declarations = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        let ty = field.ty;
        quote! { let mut #ident: ::core::option::Option<#ty> = ::core::option::Option::None; }
    });

    let arms = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        let key = field.map_key().expect("checked while collecting fields");
        let value = read_field(field);
        quote! {
            #key => {
                if #ident.is_some() {
                    return ::core::result::Result::Err(
                        ::zerocbor::Error::key_duplicated(#key),
                    );
                }
                #ident = ::core::option::Option::Some(#value);
            }
        }
    });

    // Skipping and reporting are both opt-in; without either, strict.
    let unknown = if config.allow_unknown_fields {
        quote! { _ => __reader.skip_value()?, }
    } else {
        quote! {
            other => {
                return ::core::result::Result::Err(
                    ::zerocbor::Error::unknown_variant(other),
                );
            }
        }
    };

    // Without a `default`, a missing key is an error naming it.
    let fill = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        let key = field.map_key().expect("checked while collecting fields");
        match &field.config.default {
            Default::FromTrait => quote! {
                if #ident.is_none() {
                    #ident = ::core::option::Option::Some(
                        ::core::default::Default::default(),
                    );
                }
            },
            Default::FromPath(path) => quote! {
                if #ident.is_none() {
                    #ident = ::core::option::Option::Some(#path());
                }
            },
            Default::None => quote! {
                if #ident.is_none() {
                    return ::core::result::Result::Err(
                        ::zerocbor::Error::key_not_found(#key),
                    );
                }
            },
        }
    });

    let ignored_entries = ignored.iter().map(|ident| {
        quote! { #ident: ::core::default::Default::default() }
    });
    // `fill` has already defaulted or errored, so every binding is `Some`.
    let required = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        quote! { #ident: #ident.expect("filled above") }
    });

    if !any_default && !config.allow_unknown_fields {
        // Nothing can be missing or extra, so the length is exact.
        return quote! {
            let __len = __reader.check_map_len(#count)?;
            #(#declarations)*
            let mut __index = 0;
            while __reader.next_element(__len, &mut __index)? {
                let __key = __reader.read_string()?;
                match __key.as_ref() {
                    #(#arms,)*
                    #unknown
                }
            }
            __reader.finish_map(__len)?;
            #(#fill)*
            ::core::result::Result::Ok(#name {
                #(#required,)*
                #(#ignored_entries,)*
            })
        };
    }

    quote! {
        let __len = __reader.read_map_len()?;
        #(#declarations)*
        let mut __index = 0;
        while __reader.next_element(__len, &mut __index)? {
            let __key = __reader.read_string()?;
            match __key.as_ref() {
                #(#arms,)*
                #unknown
            }
        }
        __reader.finish_map(__len)?;
        #(#fill)*
        ::core::result::Result::Ok(#name {
            #(#required,)*
            #(#ignored_entries,)*
        })
    }
}

/// Reads an enum in the chosen representation.
fn enum_body(
    config: &ContainerConfig,
    name: &Ident,
    data: &DataEnum,
    attrs: &[syn::Attribute],
) -> Result<TokenStream> {
    if config.c_enum {
        return c_enum_body(config, data, attrs);
    }
    let all_unit = data
        .variants
        .iter()
        .all(|v| matches!(v.fields, Fields::Unit));
    let any_unit = data
        .variants
        .iter()
        .any(|v| matches!(v.fields, Fields::Unit));

    // In `array` mode a fieldless variant is a bare index and a data-carrying
    // one a container, so mixing them is rejected: the reader could not tell
    // the shapes apart.
    if matches!(config.repr, Repr::Array) && any_unit && !all_unit {
        return Err(syn::Error::new_spanned(
            name,
            "in array mode a fieldless enum variant is encoded as a bare index with \
             no length head, which cannot be told apart from a data-carrying \
             variant. Make every variant carry the same shape, or use #[cbor(map)].",
        ));
    }

    // A per-variant tag decides whether the prelude looks for one at all.
    let mut any_variant_tag = false;
    for variant in &data.variants {
        if crate::repr::variant_tag(variant, None)?.is_some() {
            any_variant_tag = true;
        }
    }
    let tag_value = match config.tag {
        Some(tag) => quote! { #tag },
        None => quote! { 0 },
    };

    let mut arms = Vec::new();
    for (index, variant) in data.variants.iter().enumerate() {
        let ident = &variant.ident;
        let discriminant = index as u32;
        // The wire name, for an error about a mismatched shape to quote.
        let wire_name = crate::repr::variant_name(variant)?;
        let tag = crate::repr::variant_tag(variant, config.tag)?;
        if tag.is_some() && matches!(config.repr, Repr::Array) {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "a per-variant `tag` is not supported in array mode: the tag \
                 precedes the array but the discriminant that names the variant \
                 is inside it, so there is nothing to dispatch on. Tag the whole \
                 enum with `#[cbor(tag = N)]`, or use `#[cbor(map)]`.",
            ));
        }
        // The prelude reads the tag once and every arm checks it, so an
        // untagged variant also insists on arriving untagged.
        let check_tag = if any_variant_tag {
            let declared = match tag {
                Some(tag) => quote! { ::core::option::Option::Some(#tag) },
                None => quote! { ::core::option::Option::None },
            };
            quote! { ::zerocbor::check_optional_tag(#declared, __tag)?; }
        } else {
            quote! {}
        };
        let unit_shape = if matches!(config.repr, Repr::Map) {
            quote! {
                if __envelope.is_some() {
                    return ::core::result::Result::Err(::zerocbor::Error::unknown_variant(#wire_name));
                }
            }
        } else {
            quote! {}
        };
        let arm = match &variant.fields {
            Fields::Unit => quote! {
                #discriminant => {
                    #check_tag
                    #unit_shape
                    ::core::result::Result::Ok(Self::#ident)
                }
            },
            Fields::Named(_) | Fields::Unnamed(_) => {
                let active = util::variant_fields(&variant.fields)?;
                match config.repr {
                    // Positional, right after the index the prelude read.
                    Repr::Array => {
                        // Both shapes are built from the bindings the writes
                        // walk, so an ignored field cannot shift the rest.
                        let (entries, reads, len) = variant_array(&variant.fields, &active)?;
                        let len = len + 1;
                        // The discriminant was read after the length head.
                        let check_len = quote! {
                            if let ::zerocbor::Len::Known(__actual) = __len
                                && __actual != #len
                            {
                                return ::core::result::Result::Err(
                                    ::zerocbor::Error::ArrayLengthMismatch { expected: #len, actual: __actual },
                                );
                            }
                        };
                        quote! {
                            #discriminant => {
                                #check_tag
                                #check_len
                                #(#reads)*
                                __reader.finish_array(__len)?;
                                ::core::result::Result::Ok(Self::#ident #entries)
                            }
                        }
                    }
                    // Map mode keys a named variant's fields by name, so the
                    // count is read rather than checked. A tuple variant has no
                    // names, so its fields travel as an array.
                    Repr::Map => match &variant.fields {
                        Fields::Unnamed(_) => {
                            let (entries, reads, len) = variant_array(&variant.fields, &active)?;
                            quote! {
                                #discriminant => {
                                    #check_tag
                                    let __envelope_len = match __envelope {
                                        ::core::option::Option::Some(__len) => __len,
                                        ::core::option::Option::None => {
                                            return ::core::result::Result::Err(
                                                ::zerocbor::Error::unknown_variant(#wire_name),
                                            );
                                        }
                                    };
                                    let __len = __reader.check_array_len(#len)?;
                                    #(#reads)*
                                    __reader.finish_array(__len)?;
                                    __reader.finish_map(__envelope_len)?;
                                    ::core::result::Result::Ok(Self::#ident #entries)
                                }
                            }
                        }
                        Fields::Named(_) => {
                            let ignored = crate::repr::ignored_idents(&variant.fields)?;
                            let declarations = active.iter().map(|field| {
                                let ident = field.ident.expect("checked while collecting fields");
                                let ty = field.ty;
                                quote! {
                                    let mut #ident: ::core::option::Option<#ty> =
                                        ::core::option::Option::None;
                                }
                            });
                            let loop_body = variant_entry_loop(config, &active);
                            let fill = variant_fill(&active);
                            let construction = build_variant_from_options(ident, &active, &ignored);
                            quote! {
                                #discriminant => {
                                    #check_tag
                                    let __envelope_len = match __envelope {
                                        ::core::option::Option::Some(__len) => __len,
                                        ::core::option::Option::None => {
                                            return ::core::result::Result::Err(
                                                ::zerocbor::Error::unknown_variant(#wire_name),
                                            );
                                        }
                                    };
                                    let __len = __reader.read_map_len()?;
                                    #(#declarations)*
                                    #loop_body
                                    __reader.finish_map(__len)?;
                                    __reader.finish_map(__envelope_len)?;
                                    #fill
                                    #construction
                                }
                            }
                        }
                        Fields::Unit => unreachable!("handled by the unit arm"),
                    },
                }
            }
        };
        arms.push(arm);
    }

    // Array mode dispatches on a bare index, map mode on a text name. Either
    // way a data-carrying variant has a container in front of its fields.
    let prelude = if matches!(config.repr, Repr::Map) {
        let mut names = Vec::new();
        for (index, variant) in data.variants.iter().enumerate() {
            let name = crate::repr::variant_name(variant)?;
            let index = index as u32;
            names.push(quote! { #name => #index });
        }
        // An enum-level tag is uniform, so it is checked here; a per-variant
        // one is read and held for the arm.
        let read_tag = if config.tag.is_some() {
            quote! {
                ::zerocbor::check_optional_tag(
                    ::core::option::Option::Some(#tag_value),
                    if __reader.at_tag()? {
                        ::core::option::Option::Some(__reader.read_tag()?)
                    } else {
                        ::core::option::Option::None
                    },
                )?;
                ::core::option::Option::<u64>::None
            }
        } else if any_variant_tag {
            quote! {
                if __reader.at_tag()? {
                    ::core::option::Option::Some(__reader.read_tag()?)
                } else {
                    ::core::option::Option::None
                }
            }
        } else {
            quote! { ::core::option::Option::<u64>::None }
        };
        // A data-carrying variant sits in a one-entry map keyed by its name; a
        // fieldless one is the name alone. A map head is never a text string
        // head, so the first byte says which arrived, and the envelope length
        // rides along for the arms that expect one.
        quote! {
            // A tag precedes the envelope, so it is read first and held.
            let __tag = #read_tag;
            let __envelope = if __reader.at_map()? {
                ::core::option::Option::Some(__reader.check_map_len(1usize)?)
            } else {
                ::core::option::Option::None
            };
            let __variant_name = __reader.read_string()?;
            let __index: u32 = match __variant_name.as_ref() {
                #(#names,)*
                other => {
                    return ::core::result::Result::Err(
                        ::zerocbor::Error::unknown_variant(other),
                    );
                }
            };
        }
    } else if all_unit {
        // Without c_enum, a fieldless enum uses its declaration index.
        quote! { let __index = __reader.read_u32()?; }
    } else {
        quote! {
            let __len = __reader.read_array_len()?;
            // The discriminant is element 0, so an empty array is malformed.
            // Only the definite form has a count to check.
            if __len.known() == Some(0) {
                return ::core::result::Result::Err(
                    ::zerocbor::Error::ArrayLengthMismatch { expected: 1, actual: 0 },
                );
            }
            let __index = __reader.read_u32()?;
        }
    };

    Ok(quote! {
        #prelude
        match __index {
            #(#arms,)*
            other => ::core::result::Result::Err(
                ::zerocbor::Error::UnknownVariantIndex(other),
            ),
        }
    })
}

/// The struct-literal tail and the reads that fill it, for an array-mode variant.
///
/// A named variant builds `Self::V { .. }` and a tuple one `Self::V(..)`, so
/// the tails differ. The reads do not: each field binds in order, active from
/// the wire and ignored from `Default`.
fn variant_array(
    fields: &Fields,
    active: &[ActiveField<'_>],
) -> Result<(TokenStream, Vec<TokenStream>, usize)> {
    let declared = crate::repr::all_fields(fields)?;
    let mut reads = Vec::new();
    let mut names = Vec::new();
    for (position, field) in declared.iter().enumerate() {
        let binding = match field.ident {
            Some(ident) => quote! { #ident },
            None => {
                let ident = Ident::new(&format!("field{position}"), proc_macro2::Span::call_site());
                quote! { #ident }
            }
        };
        let ty = field.ty;
        let config = crate::repr::FieldConfig::from_syn(
            fields.iter().nth(position).expect("declared field"),
        )?;
        let value = if config.ignore {
            quote! { ::core::default::Default::default() }
        } else {
            read_field(&ActiveField {
                ident: field.ident,
                ty,
                config,
            })
        };
        names.push(binding.clone());
        reads.push(quote! {
            let #binding: #ty = #value;
        });
    }

    let entries = match fields {
        Fields::Named(_) => quote! { { #(#names),* } },
        _ => {
            let idents = names.iter();
            quote! { ( #(#idents,)* ) }
        }
    };
    Ok((entries, reads, active.len()))
}

/// The entry loop for a map-mode struct variant.
fn variant_entry_loop(config: &ContainerConfig, fields: &[ActiveField<'_>]) -> TokenStream {
    let arms = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        let key = field.map_key().expect("checked while collecting fields");
        let value = read_field(field);
        quote! {
            #key => {
                if #ident.is_some() {
                    return ::core::result::Result::Err(::zerocbor::Error::key_duplicated(#key));
                }
                #ident = ::core::option::Option::Some(#value);
            }
        }
    });
    let unknown = if config.allow_unknown_fields {
        quote! { _ => __reader.skip_value()?, }
    } else {
        quote! { other => return ::core::result::Result::Err(::zerocbor::Error::unknown_variant(other)), }
    };
    quote! {
        let mut __index = 0;
        while __reader.next_element(__len, &mut __index)? {
            let __key = __reader.read_string()?;
            match __key.as_ref() {
                #(#arms,)*
                #unknown
            }
        }
    }
}

/// The fill-or-error pass for a map-mode struct variant.
fn variant_fill(fields: &[ActiveField<'_>]) -> TokenStream {
    let fill = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        let key = field.map_key().expect("checked while collecting fields");
        match &field.config.default {
            Default::FromTrait => quote! {
                if #ident.is_none() {
                    #ident = ::core::option::Option::Some(
                        ::core::default::Default::default(),
                    );
                }
            },
            Default::FromPath(path) => quote! {
                if #ident.is_none() {
                    #ident = ::core::option::Option::Some(#path());
                }
            },
            Default::None => quote! {
                if #ident.is_none() {
                    return ::core::result::Result::Err(
                        ::zerocbor::Error::key_not_found(#key),
                    );
                }
            },
        }
    });
    quote! { #(#fill)* }
}

/// Builds a map-mode struct variant from its `Option` bindings.
fn build_variant_from_options(
    variant: &Ident,
    fields: &[ActiveField<'_>],
    ignored: &[Ident],
) -> TokenStream {
    let entries = fields.iter().map(|field| {
        let ident = field.ident.expect("named field");
        let key = field.map_key().expect("checked while collecting fields");
        quote! {
            #ident: match #ident {
                ::core::option::Option::Some(value) => value,
                ::core::option::Option::None => {
                    return ::core::result::Result::Err(
                        ::zerocbor::Error::key_not_found(#key),
                    );
                }
            }
        }
    });
    let ignored_entries = ignored.iter().map(|ident| {
        quote! { #ident: ::core::default::Default::default() }
    });
    quote! {
        ::core::result::Result::Ok(Self::#variant {
            #(#entries,)*
            #(#ignored_entries,)*
        })
    }
}

/// Reads the Rust discriminant, rather than the variant's declaration index.
fn c_enum_body(
    config: &ContainerConfig,
    data: &DataEnum,
    attrs: &[syn::Attribute],
) -> Result<TokenStream> {
    let repr = util::enum_integer_repr(attrs)?;
    let arms = data.variants.iter().map(|variant| {
        let ident = &variant.ident;
        quote! {
            __value if ::core::convert::TryFrom::try_from(Self::#ident as #repr).ok() == ::core::option::Option::<i128>::Some(__value) =>
                ::core::result::Result::Ok(Self::#ident)
        }
    });
    Ok(with_tag(
        config.tag,
        quote! {
            let __discriminant = __reader.read_integer()?;
            match __discriminant {
                #(#arms,)*
                other => ::core::result::Result::Err(::zerocbor::Error::UnknownVariantDiscriminant(other)),
            }
        },
    ))
}
