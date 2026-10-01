use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Result};

use crate::repr::{ActiveField, ContainerConfig, Repr};
use crate::util;

/// Expands `ToCbor` for a struct or an enum.
pub fn expand(input: &DeriveInput) -> Result<TokenStream> {
    let config = ContainerConfig::from_attrs(input)?;
    let name = &input.ident;
    let generics = util::add_trait_bounds(&input.generics, false);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => struct_body(&config, data)?,
        Data::Enum(data) => enum_body(&config, data, &input.attrs)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "ToCbor cannot be derived for a union",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics ::zerocbor::ToCbor for #name #ty_generics #where_clause {
            #[inline]
            fn write<__W: ::zerocbor::Write>(
                &self,
                __writer: &mut __W,
            ) -> ::zerocbor::Result<()> {
                #body
            }
        }
    })
}

/// Writes a struct in the chosen representation.
///
/// In `array` mode `key = N` places a field at a position and any gap is filled
/// with `null`. Without it, fields sit in declaration order and no gaps arise.
fn struct_body(config: &ContainerConfig, data: &syn::DataStruct) -> Result<TokenStream> {
    // A newtype is its inner value, not a one-element container. That is also
    // the shape a tag needs: RFC 8949's bignum and embedded-CBOR tags put the
    // tag directly in front of that value.
    if matches!(data.fields, Fields::Unnamed(_)) {
        return newtype_body(config, data);
    }
    let fields = util::struct_fields(data)?;
    let body = match config.repr {
        Repr::Array => {
            let mut writes: Vec<TokenStream> = Vec::new();
            for (position, field) in fields.iter().enumerate() {
                let accessor = field.accessor(position);
                match field.array_index()? {
                    None => writes.push(write_field_at(field, accessor)),
                    Some(target) => {
                        // The gap in between is `null`, so the wire indices
                        // are the ones asked for, not the written order.
                        let previous = writes.len();
                        for _ in previous..target {
                            writes.push(quote! { __writer.write_null()?; });
                        }
                        if target < previous {
                            return Err(syn::Error::new_spanned(
                                field.ty,
                                format!(
                                    "`key = {target}` is out of order; array keys must \
                                         increase so the wire positions stay consistent"
                                ),
                            ));
                        }
                        writes.push(write_field_at(field, accessor));
                    }
                }
            }
            let len = writes.len();
            quote! {
                __writer.write_array_len(#len)?;
                #(#writes)*
                Ok(())
            }
        }
        Repr::Map => map_writes(&fields, |position, field| field.accessor(position))?,
    };
    Ok(with_tag(config.tag, body))
}

/// Writes a newtype as its inner value, under a tag if one was declared.
fn newtype_body(config: &ContainerConfig, data: &syn::DataStruct) -> Result<TokenStream> {
    let field = crate::repr::newtype_field(data)?;
    let active = crate::repr::newtype_active_field(field)?;
    let body = if active.config.ignore {
        quote! { __writer.write_null() }
    } else {
        // Positional, so a newtype reads out of `self.0`. A field write is a
        // statement in a struct body but a whole value here, so it is closed off.
        let write = write_field_at(&active, quote! { &self.0 });
        quote! {
            {
                #write
                ::zerocbor::Result::Ok(())
            }
        }
    };
    Ok(with_tag(config.tag, body))
}

/// Builds one enum arm, writing a tag in front of the body if there is one.
///
/// The tag goes inside the arm: it applies to the value, not the match.
fn variant_arm(tag: Option<u64>, pattern: TokenStream, body: TokenStream) -> TokenStream {
    match tag {
        None => quote! { #pattern => { #body } },
        Some(tag) => quote! {
            #pattern => {
                __writer.write_tag(#tag)?;
                #body
            }
        },
    }
}

/// Puts a tag in front of a value, if the type declared one.
///
/// A tag precedes its value, so it goes before the body.
fn with_tag(tag: Option<u64>, body: TokenStream) -> TokenStream {
    match tag {
        None => body,
        Some(tag) => quote! {
            __writer.write_tag(#tag)?;
            #body
        },
    }
}

/// Writes a map-mode container's entries, keyed and in canonical order.
///
/// The order RFC 8949 Section 4.2.1 gives a map's keys, decided at expansion
/// time so it costs nothing at run time. Each field is paired with its
/// accessor position, so sorting moves the pair and an ignored field cannot
/// shift the next one's accessor.
fn map_writes<F>(fields: &[ActiveField<'_>], accessor: F) -> Result<TokenStream>
where
    F: Fn(usize, &ActiveField<'_>) -> TokenStream,
{
    // The key comes first, because the sort is on it: a renamed field sorts
    // under its wire name, not its identifier.
    let mut entries: Vec<(String, usize, &ActiveField<'_>)> = fields
        .iter()
        .enumerate()
        .map(|(position, field)| Ok((field.map_key()?, position, field)))
        .collect::<Result<_>>()?;
    entries.sort_by(|a, b| crate::repr::canonical_key_order(&a.0, &b.0));

    let mut writes = Vec::with_capacity(entries.len());
    for (key, position, field) in &entries {
        let value = write_field_at(field, accessor(*position, field));
        writes.push(quote! {
            __writer.write_string(#key)?;
            #value
        });
    }
    let len = writes.len();
    Ok(quote! {
        __writer.write_map_len(#len)?;
        #(#writes)*
        Ok(())
    })
}

/// Writes one field through the accessor at `position`.
fn write_field_at(field: &ActiveField<'_>, accessor: TokenStream) -> TokenStream {
    if !field.is_bytes() {
        let ty = field.ty;
        return quote! {
            <#ty as ::zerocbor::ToCbor>::write(&#accessor, __writer)?;
        };
    }
    quote! {
        __writer.write_binary(
            ::core::convert::AsRef::<[u8]>::as_ref(&#accessor),
        )?;
    }
}

/// Writes an enum in the chosen representation.
///
/// A fieldless variant is its index in array mode and its name in map mode,
/// which is an externally tagged enum as a serde-based encoder sees it. A
/// data-carrying variant is wrapped: array mode puts the index in element 0,
/// map mode makes a one-entry map keyed by the name.
fn enum_body(
    config: &ContainerConfig,
    data: &syn::DataEnum,
    attrs: &[syn::Attribute],
) -> Result<TokenStream> {
    if config.c_enum {
        let repr = util::enum_integer_repr(attrs)?;
        let arms = data.variants.iter().map(|variant| {
            let ident = &variant.ident;
            quote! {
                Self::#ident => {
                    let __value = <i128 as ::core::convert::TryFrom<#repr>>::try_from(Self::#ident as #repr)
                        .map_err(|_| ::zerocbor::Error::IntegerOutOfRange)?;
                    if __value >= 0 {
                        let __argument = <u64 as ::core::convert::TryFrom<i128>>::try_from(__value)
                            .map_err(|_| ::zerocbor::Error::IntegerOutOfRange)?;
                        __writer.write_u64(__argument)
                    } else {
                        let __argument = <u64 as ::core::convert::TryFrom<i128>>::try_from(-1 - __value)
                            .map_err(|_| ::zerocbor::Error::IntegerOutOfRange)?;
                        __writer.write_negative(__argument)
                    }
                }
            }
        });
        return Ok(with_tag(config.tag, quote! { match self { #(#arms,)* } }));
    }
    let mut arms = Vec::new();

    for (index, variant) in data.variants.iter().enumerate() {
        let ident = &variant.ident;
        let discriminant = index as u32;
        // A variant's `key` renames it in map mode; array mode uses position.
        let name = crate::repr::variant_name(variant)?;
        let tag = crate::repr::variant_tag(variant, config.tag)?;

        let arm = match &variant.fields {
            // Its index in array mode, its name as a text string in map mode.
            Fields::Unit if matches!(config.repr, Repr::Array) => variant_arm(
                tag,
                quote! { Self::#ident },
                quote! { __writer.write_u32(#discriminant) },
            ),
            Fields::Unit => variant_arm(
                tag,
                quote! { Self::#ident },
                quote! { __writer.write_string(#name) },
            ),
            fields => {
                // The fields bind so the body reads them off the matched
                // reference. Binding and writes walk one list, or an ignored
                // field shifts the rest.
                let active = util::variant_fields(fields)?;
                let binding = variant_binding(fields, &active)?;
                let writes_repr = match (config.repr, fields) {
                    // No names to key by, so an array even in map mode.
                    (Repr::Map, Fields::Unnamed(_)) => Repr::Array,
                    (repr, _) => repr,
                };
                // A named variant's fields are a map in their own right, keyed
                // and ordered as a struct's, so built as one piece.
                let fields_map = match (config.repr, fields) {
                    (Repr::Map, Fields::Named(_)) => Some(map_writes(&active, |_, field| {
                        let ident = field.ident.expect("a named variant's fields have names");
                        quote! { #ident }
                    })?),
                    _ => None,
                };
                let writes = variant_writes(writes_repr, &active, fields)?;
                let len = active.len() + 1;
                match config.repr {
                    Repr::Array => variant_arm(
                        tag,
                        quote! { Self::#ident #binding },
                        quote! {
                            __writer.write_array_len(#len)?;
                            __writer.write_u32(#discriminant)?;
                            #(#writes)*
                            Ok(())
                        },
                    ),
                    // The name keys the envelope map and the fields are its
                    // value. Flattening them into one map would leave the
                    // name's own value unwritten.
                    Repr::Map => {
                        let payload = match &fields_map {
                            Some(body) => quote! { #body },
                            None => {
                                let payload_len = active.len();
                                quote! {
                                    __writer.write_array_len(#payload_len)?;
                                    #(#writes)*
                                    Ok(())
                                }
                            }
                        };
                        variant_arm(
                            tag,
                            quote! { Self::#ident #binding },
                            quote! {
                                __writer.write_map_len(1usize)?;
                                __writer.write_string(#name)?;
                                #payload
                            },
                        )
                    }
                }
            }
        };
        arms.push(arm);
    }

    Ok(quote! {
        match self {
            #(#arms,)*
        }
    })
}

/// The pattern that binds a variant's fields.
///
/// The match is against `&Self`, so match ergonomics already bind by reference
/// and an explicit `ref` would be rejected. A named variant is bound with a
/// braced pattern and a tuple variant with a parenthesized one, matching the
/// declaration.
fn variant_binding(fields: &Fields, active: &[ActiveField<'_>]) -> Result<TokenStream> {
    match fields {
        Fields::Unit => Ok(TokenStream::new()),
        Fields::Named(_) => {
            let mut names = Vec::new();
            for field in active {
                let ident = field.ident.expect("named field");
                names.push(quote! { #ident });
            }
            Ok(quote! { { #(#names,)* .. } })
        }
        Fields::Unnamed(unnamed) => {
            // Positional fields bind to generated names, indexed by their
            // position in the declaration rather than in `active`, so the
            // binding stays aligned with `self.0`, `self.1`, and so on.
            let mut names = Vec::new();
            for (position, field) in unnamed.unnamed.iter().enumerate() {
                if crate::repr::FieldConfig::from_syn(field)?.ignore {
                    names.push(quote! { _ });
                    continue;
                }
                let ident =
                    syn::Ident::new(&format!("field{position}"), proc_macro2::Span::call_site());
                names.push(quote! { #ident });
            }
            Ok(quote! { ( #(#names),* ) })
        }
    }
}

/// The write expression for each of a variant's fields.
///
/// A positional field has no name of its own, so the binding is named after its
/// position. Positions come from the active subset in order, which is the
/// declaration order minus the ignored fields.
fn variant_writes(
    repr: Repr,
    active: &[ActiveField<'_>],
    fields: &Fields,
) -> Result<Vec<TokenStream>> {
    let mut positions = Vec::new();
    for (position, field) in fields.iter().enumerate() {
        if !crate::repr::FieldConfig::from_syn(field)?.ignore {
            positions.push(position);
        }
    }
    let mut cursor = 0usize;
    let mut writes = Vec::new();
    for field in active {
        let value = match field.ident {
            Some(ident) => quote! { #ident },
            None => {
                // Every positional field that is not ignored is on the wire, in
                // declaration order, so a running position is what identifies it.
                let position = positions[cursor];
                cursor += 1;
                let ident =
                    syn::Ident::new(&format!("field{position}"), proc_macro2::Span::call_site());
                quote! { #ident }
            }
        };
        if matches!(repr, Repr::Map) {
            let key = field.map_key()?;
            writes.push(quote! { __writer.write_string(#key)?; });
        }
        if field.is_bytes() {
            writes.push(quote! {
                __writer.write_binary(
                    ::core::convert::AsRef::<[u8]>::as_ref(#value),
                )?;
            });
        } else {
            let ty = field.ty;
            writes.push(quote! {
                <#ty as ::zerocbor::ToCbor>::write(#value, __writer)?;
            });
        }
    }
    Ok(writes)
}
