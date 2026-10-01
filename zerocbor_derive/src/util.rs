use syn::{DataStruct, Error, Fields, GenericParam, Generics, Result, parse_quote};

use crate::repr::ActiveField;

/// Collects the non-skipped fields of a named-field struct.
pub fn struct_fields(data: &DataStruct) -> Result<Vec<ActiveField<'_>>> {
    match &data.fields {
        Fields::Named(named) => crate::repr::active_fields(named),
        other => Err(Error::new_spanned(
            other,
            "only named-field structs are supported; write the impl by hand for \
             a tuple struct",
        )),
    }
}

/// Adds `T: ToCbor` or `T: FromCbor<'de>` to every type parameter.
///
/// A parameter that already names one of the traits is left alone.
pub fn add_trait_bounds(generics: &Generics, decoding: bool) -> Generics {
    let mut generics = generics.clone();
    let has_cbor = {
        let clause = generics.make_where_clause();
        clause.predicates.iter().any(mentions_cbor_trait)
    };
    if has_cbor {
        return generics;
    }

    let type_params: Vec<syn::Ident> = generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(type_param) => Some(type_param.ident.clone()),
            _ => None,
        })
        .collect();

    let clause = generics.make_where_clause();
    for ident in type_params {
        // `'de: FromCbor<'de>` names the input lifetime, which a plain
        // `TypeParamBound` cannot express, so it goes in the predicates.
        let bound: syn::WherePredicate = if decoding {
            parse_quote!(#ident: ::zerocbor::FromCbor<'de>)
        } else {
            parse_quote!(#ident: ::zerocbor::ToCbor)
        };
        clause.predicates.push(bound);
    }
    generics
}

/// Whether a where-predicate already mentions one of the crate's traits.
fn mentions_cbor_trait(predicate: &syn::WherePredicate) -> bool {
    let syn::WherePredicate::Type(pred) = predicate else {
        return false;
    };
    pred.bounds.iter().any(|bound| match bound {
        syn::TypeParamBound::Trait(t) => t
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "ToCbor" || segment.ident == "FromCbor"),
        _ => false,
    })
}

/// Declares the `'de` lifetime that `FromCbor<'de>` requires.
///
/// Reading `&'a str` out of an input living for `'de` needs `'de: 'a`.
pub fn add_de_lifetime(generics: &Generics) -> Generics {
    let mut generics = generics.clone();
    if generics
        .params
        .iter()
        .any(|param| matches!(param, GenericParam::Lifetime(l) if l.lifetime.ident == "de"))
    {
        return generics;
    }

    // Each of the type's lifetimes records `'de: 'a`.
    let existing: Vec<syn::Lifetime> = generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Lifetime(lifetime) => Some(lifetime.lifetime.clone()),
            _ => None,
        })
        .collect();

    let clause = generics.make_where_clause();
    for lifetime in existing {
        let bound: syn::WherePredicate = parse_quote!('de: #lifetime);
        clause.predicates.push(bound);
    }

    generics.params.insert(0, parse_quote!('de));
    generics
}

/// The non-ignored fields of a variant, which is what goes on the wire.
pub fn variant_fields(fields: &syn::Fields) -> Result<Vec<crate::repr::ActiveField<'_>>> {
    match fields {
        Fields::Named(named) => crate::repr::active_fields(named),
        Fields::Unnamed(unnamed) => {
            let mut out = Vec::with_capacity(unnamed.unnamed.len());
            for field in &unnamed.unnamed {
                let config = crate::repr::FieldConfig::from_syn(field)?;
                if config.ignore {
                    continue;
                }
                out.push(crate::repr::ActiveField {
                    ident: None,
                    ty: &field.ty,
                    config,
                });
            }
            Ok(out)
        }
        Fields::Unit => Ok(Vec::new()),
    }
}

/// A casting type that preserves the enum's integer discriminants.
pub fn enum_integer_repr(attrs: &[syn::Attribute]) -> Result<syn::Type> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("repr")) {
        let args = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        )?;
        for arg in args {
            if let syn::Meta::Path(path) = arg
                && let Some(ident) = path.get_ident()
                && matches!(
                    ident.to_string().as_str(),
                    "u8" | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                )
            {
                return Ok(syn::Type::Path(syn::TypePath { qself: None, path }));
            }
        }
    }
    Ok(parse_quote!(i128))
}
