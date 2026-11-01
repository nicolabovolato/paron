mod values;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Field, Fields, Lit, Meta, Token, parse_macro_input,
};

/// Names every argument of a clap `Args` struct, as one nested constant.
///
/// `Thing::NAMES.port` is the argument declared on `Thing`. A `#[command(flatten)]` field nests,
/// so an argument clap flattened in is reached through the struct that holds it:
/// `TunnelBootstrapArgs::NAMES.served.ports`.
#[proc_macro_derive(ArgNames)]
pub fn arg_names(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let Data::Struct(data) = &input.data else {
        return error(&input, "ArgNames applies to a struct");
    };

    let Fields::Named(fields) = &data.fields else {
        return error(&input, "ArgNames needs named fields");
    };

    let arguments: Vec<_> = fields.named.iter().filter_map(name_of).collect();
    let declarations = arguments.iter().map(|(declaration, _)| declaration);
    let values = arguments.iter().map(|(_, value)| value);

    let derived = &input.ident;
    let names = format_ident!("{derived}Names");
    let (implementing, generics, where_clause) = input.generics.split_for_impl();

    quote! {
        #[derive(Clone, Copy, Debug)]
        pub struct #names {
            #(#declarations,)*
        }

        impl #implementing #derived #generics #where_clause {
            pub const NAMES: #names = #names {
                #(#values,)*
            };
        }
    }
    .into()
}

/// The field of the names struct, and the value it takes.
fn name_of(field: &Field) -> Option<(proc_macro2::TokenStream, proc_macro2::TokenStream)> {
    let name = field.ident.as_ref()?;
    let cfgs: Vec<_> = cfgs(field).collect();

    if let Some(command) = attribute(field, "command").map(keys) {
        if !named(&command, "flatten") {
            return None;
        }

        let nested = nested_names(&field.ty)?;
        let held = &field.ty;

        return Some((
            quote!(#(#cfgs)* pub #name: #nested),
            quote!(#(#cfgs)* #name: <#held>::NAMES),
        ));
    }

    let argument = attribute(field, "arg").map(keys)?;
    let flag = format!("--{}", long(&argument, &name.to_string())?);
    let variable = text(&argument, "env").map_or_else(
        || quote!(::core::option::Option::None),
        |variable| quote!(::core::option::Option::Some(#variable)),
    );

    Some((
        quote!(#(#cfgs)* pub #name: ::clap_extra::ArgName),
        quote! {
            #(#cfgs)* #name: ::clap_extra::ArgName { flag: #flag, env: #variable }
        },
    ))
}

/// The names struct of a flattened field, named after the struct it holds.
fn nested_names(ty: &syn::Type) -> Option<proc_macro2::TokenStream> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let mut path = path.path.clone();
    let last = path.segments.last_mut()?;

    last.ident = format_ident!("{}Names", last.ident);

    Some(quote!(#path))
}

/// clap turns a bare `long` into the field name with each underscore as a dash. An explicit
/// `long = "..."` wins. A field with no `long` is positional and has no flag.
fn long(argument: &Keys, field: &str) -> Option<String> {
    if let Some(given) = text(argument, "long") {
        return Some(given);
    }

    named(argument, "long").then(|| field.replace('_', "-"))
}

type Keys = Punctuated<Meta, Token![,]>;

fn keys(attribute: &Attribute) -> Keys {
    attribute
        .parse_args_with(Keys::parse_terminated)
        .unwrap_or_default()
}

fn text(argument: &Keys, key: &str) -> Option<String> {
    argument.iter().find_map(|meta| match meta {
        Meta::NameValue(pair) if pair.path.is_ident(key) => match &pair.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            }) => Some(text.value()),
            _ => None,
        },
        _ => None,
    })
}

fn named(argument: &Keys, key: &str) -> bool {
    argument
        .iter()
        .any(|meta| matches!(meta, Meta::Path(path) if path.is_ident(key)))
}

fn attribute<'a>(field: &'a Field, name: &str) -> Option<&'a Attribute> {
    field.attrs.iter().find(|next| next.path().is_ident(name))
}

/// The `#[cfg(...)]` of a field, which everything generated for it must carry: a field that a
/// platform leaves out has no name and no value there.
fn cfgs(field: &Field) -> impl Iterator<Item = &Attribute> {
    field
        .attrs
        .iter()
        .filter(|next| next.path().is_ident("cfg"))
}

fn error(input: &DeriveInput, message: &str) -> TokenStream {
    syn::Error::new_spanned(&input.ident, message)
        .to_compile_error()
        .into()
}

/// Reads a clap `Args` value back as the arguments it would have been parsed from, under either
/// of the names those arguments answer to.
///
/// A field that was not given is left out. A `#[command(flatten)]` field contributes whatever the
/// struct it holds reads back.
#[proc_macro_derive(ArgValues)]
pub fn arg_values(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let Data::Struct(data) = &input.data else {
        return error(&input, "ArgValues applies to a struct");
    };

    let Fields::Named(fields) = &data.fields else {
        return error(&input, "ArgValues needs named fields");
    };

    let words = fields
        .named
        .iter()
        .filter_map(|field| read_of(field, Reading::Flags));
    let pairs = fields
        .named
        .iter()
        .filter_map(|field| read_of(field, Reading::Environment));

    let derived = &input.ident;
    let (implementing, generics, where_clause) = input.generics.split_for_impl();

    quote! {
        impl #implementing #derived #generics #where_clause {
            /// Command line words, an argument that takes no value named alone
            pub fn flags(&self) -> ::std::vec::Vec<::std::string::String> {
                let mut words = ::std::vec::Vec::new();

                #(#words)*

                words
            }

            /// What a service definition sets. `true` is the one value clap reads back as a flag
            /// that was given.
            pub fn environment(
                &self,
            ) -> ::std::vec::Vec<(::std::string::String, ::std::string::String)> {
                let mut values = ::std::vec::Vec::new();

                #(#pairs)*

                values
            }
        }
    }
    .into()
}

#[derive(Clone, Copy)]
enum Reading {
    Flags,
    Environment,
}

fn read_of(field: &Field, reading: Reading) -> Option<proc_macro2::TokenStream> {
    let name = field.ident.as_ref()?;
    let cfgs: Vec<_> = cfgs(field).collect();

    if let Some(command) = attribute(field, "command").map(keys) {
        if !named(&command, "flatten") {
            return None;
        }

        return Some(match reading {
            Reading::Flags => quote! { #(#cfgs)* words.extend(self.#name.flags()); },
            Reading::Environment => quote! { #(#cfgs)* values.extend(self.#name.environment()); },
        });
    }

    let argument = attribute(field, "arg").map(keys)?;

    long(&argument, &name.to_string())?;
    let variable = text(&argument, "env")?;

    let given = values::given(&field.ty, &quote!((&self.#name)));

    Some(match reading {
        Reading::Flags => quote! {
            #(#cfgs)*
            {
                if let Some(value) = #given {
                    words.push(Self::NAMES.#name.flag.to_owned());

                    if let Some(value) = value {
                        words.push(value);
                    }
                }
            }
        },
        Reading::Environment => quote! {
            #(#cfgs)*
            {
                if let Some(value) = #given {
                    values.push((
                        #variable.to_owned(),
                        value.unwrap_or_else(|| "true".to_owned()),
                    ));
                }
            }
        },
    })
}
