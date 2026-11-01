use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericArgument, PathArguments, Type};

/// Reads one field as `None` where it was not given, `Some(None)` where it takes no value, and
/// `Some(Some(text))` otherwise.
///
/// The shape is taken from the type rather than from a trait, because the trait would have to be
/// implemented for `SocketAddr` and `url::Url`, which the orphan rule puts out of reach.
pub fn given(ty: &Type, value: &TokenStream) -> TokenStream {
    if let Some(inner) = wrapped(ty, "Option") {
        let inner = given(inner, &quote!(inner));

        return quote! {
            match #value {
                Some(inner) => #inner,
                None => None,
            }
        };
    }

    if let Some(inner) = wrapped(ty, "Vec") {
        let text = text(inner, &quote!(item));

        return quote! {
            if #value.is_empty() {
                None
            } else {
                Some(Some(
                    #value
                        .iter()
                        .map(|item| #text)
                        .collect::<::std::vec::Vec<_>>()
                        .join(","),
                ))
            }
        };
    }

    if named(ty, "bool") {
        return quote! {
            if *#value { Some(None) } else { None }
        };
    }

    let text = text(ty, value);

    quote!(Some(Some(#text)))
}

/// One value as the string clap would have parsed it from. `PathBuf` is the one type here that
/// carries no `Display`.
fn text(ty: &Type, value: &TokenStream) -> TokenStream {
    if named(ty, "PathBuf") {
        return quote!(#value.display().to_string());
    }

    quote!(#value.to_string())
}

fn wrapped<'a>(ty: &'a Type, outer: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;

    if last.ident != outer {
        return None;
    }

    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };

    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    })
}

fn named(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path)
        if path.path.segments.last().is_some_and(|last| last.ident == name))
}
