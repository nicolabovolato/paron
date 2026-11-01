//! The attribute behind [`js_frontend_embed::embed`], which is where the documentation is.
//!
//! Nothing here is meant to be depended on directly: the parent crate re-exports the one macro,
//! and the code it expands to names the parent by path.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, ItemStruct, LitStr, Token, parse_macro_input};

/// What Vite writes, and what has to be kept out of the bundle. See [`Options::manifest`].
const VITE_MANIFEST: &str = ".vite/*";

/// Everything the attribute accepts.
struct Options {
    /// The bundler's manifest, as a pattern rather than a name: a compressed store keeps it under
    /// one more extension than it was written with.
    manifest: Option<LitStr>,
}

impl Parse for Options {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut manifest = None;

        while !input.is_empty() {
            let option: Ident = input.parse()?;

            match option.to_string().as_str() {
                "manifest" => {
                    input.parse::<Token![=]>()?;
                    manifest = Some(input.parse()?);
                }
                unknown => {
                    return Err(syn::Error::new(
                        option.span(),
                        format!(
                            "`{unknown}` is not an option of `embed`. It takes `manifest = \
                             \"pattern\"`"
                        ),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self { manifest })
    }
}

/// The frontend the build script published, embedded and ready to serve.
///
/// See the parent crate for what this expands to and why.
#[proc_macro_attribute]
pub fn embed(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let options = parse_macro_input!(attribute as Options);
    let bundle = parse_macro_input!(item as ItemStruct);

    expand(&options, &bundle)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(options: &Options, bundle: &ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    if !matches!(bundle.fields, syn::Fields::Unit) {
        return Err(syn::Error::new_spanned(
            &bundle.fields,
            "`rust_embed` derives only for a unit struct, so this goes on one: `struct Bundle;`",
        ));
    }

    let name = &bundle.ident;
    let visibility = &bundle.vis;
    let attributes = &bundle.attrs;

    let manifest = options
        .manifest
        .clone()
        .unwrap_or_else(|| LitStr::new(VITE_MANIFEST, name.span()));

    // Named after the bundle, so two frontends in one module do not collide.
    let guard = format_ident!("embed_excludes_the_manifest_of_{}", to_snake(name));

    Ok(quote! {
        #(#attributes)*
        #[derive(::js_frontend_embed::Embed)]
        #[folder = "$JS_FRONTEND_EMBED_STORE"]
        #[crate_path = "js_frontend_embed::rust_embed"]
        // A literal, because `rust_embed` ignores an `exclude` that is not one rather than
        // rejecting it. `#guard` below checks that this held.
        #[exclude = #manifest]
        #visibility struct #name;

        impl ::js_frontend_embed::Frontend for #name {
            const DIRECTORY: &'static str = ::core::env!(
                "JS_FRONTEND_EMBED_DIR",
                "JS_FRONTEND_EMBED_DIR is unset: the build script has to call `Build::run` first"
            );

            const ENVIRONMENT: &'static str = ::core::env!(
                "JS_FRONTEND_EMBED_ENV",
                "JS_FRONTEND_EMBED_ENV is unset: the build script has to call `Build::run` first"
            );
        }

        impl #name {
            /// The frontend's source directory, which is where a dev server is started.
            pub fn directory() -> &'static ::std::path::Path {
                ::std::path::Path::new(
                    <Self as ::js_frontend_embed::Frontend>::DIRECTORY,
                )
            }

            /// The names the bundler emitted with a content hash, which are the ones that may be
            /// cached forever.
            pub fn fingerprinted() -> impl ::core::iter::Iterator<Item = &'static str> {
                ::js_frontend_embed::names(::core::env!(
                    "JS_FRONTEND_EMBED_FINGERPRINTED",
                    "JS_FRONTEND_EMBED_FINGERPRINTED is unset: the build script has to call \
                     `Build::run` first"
                ))
            }

            /// Where the bundler's manifest was looked for, or `None` if the build script named
            /// none.
            pub fn manifest() -> ::core::option::Option<&'static str> {
                ::core::option_env!("JS_FRONTEND_EMBED_MANIFEST")
            }

            /// The route that answers from this bundle, caching what the bundler fingerprinted.
            pub fn service() -> ::js_frontend_embed::axum::routing::MethodRouter {
                ::js_frontend_embed::embedded::<Self>(::js_frontend_embed::Policy::holding(
                    Self::fingerprinted(),
                ))
            }
        }

        // `manifest()` rather than the checked read above, because a build that named no manifest
        // has nothing to check and still has to compile.
        #[cfg(test)]
        #[test]
        fn #guard() {
            if let ::core::option::Option::Some(path) = #name::manifest() {
                let compressed = ::std::format!("{path}.br");

                assert!(
                    <#name as ::js_frontend_embed::rust_embed::Embed>::get(path).is_none()
                        && <#name as ::js_frontend_embed::rust_embed::Embed>::get(&compressed)
                            .is_none(),
                    "{path} was embedded, so the bundler's manifest is served under the source \
                     paths it is keyed by. `embed`'s `manifest` does not cover it."
                );
            }
        }
    })
}

/// `AdminUi` to `admin_ui`, so a generated test reads like a hand-written one.
fn to_snake(name: &Ident) -> String {
    let name = name.to_string();
    let mut snake = String::with_capacity(name.len());

    for (position, letter) in name.char_indices() {
        if letter.is_uppercase() && position > 0 {
            snake.push('_');
        }

        snake.extend(letter.to_lowercase());
    }

    snake
}
