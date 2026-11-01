//! The attribute behind `serde-utoipa-validate`.
//!
//! It exists because serde and utoipa each infer the wrong thing about a field that may be absent,
//! and correcting them by hand means two attributes on every field. Rewriting the fields here
//! means the corrections are emitted rather than remembered.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Field, Fields, Ident, Type, parse_macro_input};

/// Reads a body without giving up on the first bad field: what arrives, what it becomes, and
/// how the two are checked.
///
/// Applies to a request body and to any type nested inside one, which is why it is not called
/// `request`.
///
/// Every field is read through [`Patch`](::serde_utoipa_validate::Patch), so an absent one is a
/// rejection this can report rather than a serde error that ends the body — and the document is
/// told the field is required anyway.
///
/// How a field is spelled decides what it means:
///
/// | written | may be omitted | `null` |
/// |---|---|---|
/// | `T` | no | refused by `T` |
/// | `Option<T>` | yes | reads as absent, which is what `Option` means to serde |
/// | `Patch<T>` | yes | refused by `T` |
/// | `Patch<Option<T>>` | yes | a value, so the field can be cleared |
///
/// A field marked `#[validate(default)]` may also be omitted, and its own type says what an
/// absent one becomes by implementing [`Absent`](::serde_utoipa_validate::Absent). It is written
/// without an `Option`, the twin holds the value rather than an `Option` of it, and the document
/// calls it optional. A type that does not implement `Absent` is a compile error there.
///
/// `#[validate(default = value)]` does the same for this field only: `value` is the parsed value,
/// for example a variant of a domain enum, and the document shows it in its sent form. The field's
/// type must then be one that a response can write.
///
/// # What it emits
///
/// `#[serde(default)]` and `#[schema(value_type = …)]` on every field; a twin named `{Name}Parsed`
/// holding the judged types; [`Wire`](::serde_utoipa_validate::Wire); and an inherent `validated`,
/// which is how a handler gets the twin and the only way to reach one.
///
/// Pass a name — `#[validate(UpdateUser)]` — to call the twin something other than `{Name}Parsed`.
///
/// A struct that also derives `IntoParams` is read from a query string. utoipa reads `#[param]`
/// there and never `#[schema]`, so both are written, and the parameter states its schema inline:
/// only a body or a response registers a component, so a parameter that referenced one would
/// reference nothing.
///
/// It emits no derives. `Deserialize` and `ToSchema` are the caller's to write, because the
/// attributes above are their helpers; declaring the type is the caller's job and only the
/// rewriting is this macro's. Anything else the caller writes — `#[serde(tag = …)]`,
/// `#[schema(inline)]` — is carried through.
///
/// It has to be an attribute rather than a derive because it rewrites the fields, which a derive
/// cannot do.
#[proc_macro_attribute]
pub fn validate(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let parsed_name = if attribute.is_empty() {
        None
    } else {
        Some(parse_macro_input!(attribute as Ident))
    };

    let input = parse_macro_input!(item as DeriveInput);

    match expand(&input, parsed_name.as_ref()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// What a struct or an enum contributes: how it is read, what its twin holds, and how it is judged.
struct Shape {
    read: proc_macro2::TokenStream,
    twin: proc_macro2::TokenStream,
    walk: proc_macro2::TokenStream,
    /// The functions that give the document the sent form of each `#[validate(default = value)]`.
    defaults: Vec<proc_macro2::TokenStream>,
}

fn expand(
    input: &DeriveInput,
    parsed_name: Option<&Ident>,
) -> syn::Result<proc_macro2::TokenStream> {
    required_derives(input)?;

    // A struct that derives `IntoParams` is read from a query string, and utoipa reads `#[param]`
    // there rather than `#[schema]`. Both are emitted so one field spelling serves both.
    let params = derived(input).iter().any(|derive| derive == "IntoParams");

    let name = &input.ident;
    let parsed = parsed_name
        .cloned()
        .unwrap_or_else(|| format_ident!("{name}Parsed"));
    let visibility = &input.vis;

    // Derives first: the attributes they carry — `#[serde(tag = …)]` and friends — are their
    // helpers, and a helper written before its derive does not compile.
    let (derives, rest): (Vec<_>, Vec<_>) = input
        .attrs
        .iter()
        .partition(|attribute| attribute.path().is_ident("derive"));

    let (keyword, shape) = match &input.data {
        Data::Struct(body) => (quote!(struct), fields(&body.fields, name, &parsed, params)?),
        Data::Enum(body) => (quote!(enum), variants(body, name, &parsed, params)?),
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "#[validate] takes a struct or an enum",
            ));
        }
    };

    let Shape {
        read,
        twin,
        walk,
        defaults,
    } = shape;

    Ok(quote! {
        #(#derives)*
        #(#rest)*
        #visibility #keyword #name #read

        /// The same, with every field judged. Only reachable by judging one.
        ///
        /// No `Debug`: these are the parsed domain types, and some of them are secrets.
        #visibility #keyword #parsed #twin

        impl #name {
            /// Everything judged, or everything that was wrong with it. What a handler calls.
            ///
            /// Inherent rather than a method on `Wire`, so a caller needs no trait in scope.
            #visibility fn validated(
                self,
            ) -> ::std::result::Result<
                #parsed,
                ::serde_utoipa_validate::ValidationErrors,
            > {
                let mut errors = ::serde_utoipa_validate::ValidationErrors::new();
                let parsed =
                    ::serde_utoipa_validate::Wire::parse(self, &mut errors);

                errors.build(|| parsed)
            }
        }

        impl #name {
            #(#defaults)*
        }

        impl ::serde_utoipa_validate::Wire for #name {
            type Parsed = #parsed;

            fn parse(
                self,
                errors: &mut ::serde_utoipa_validate::ValidationErrors,
            ) -> ::std::option::Option<#parsed> {
                #walk
            }
        }
    })
}

fn fields(body: &Fields, name: &Ident, parsed: &Ident, params: bool) -> syn::Result<Shape> {
    let fields = named(body, body)?;
    let read = fields
        .iter()
        .copied()
        .map(|field| read_field(field, params, name, None));
    let defaults = fields
        .iter()
        .copied()
        .filter_map(|field| default_function(field, None))
        .collect();
    let hold = fields.iter().copied().map(hold_field);
    let judge = fields.iter().copied().map(judge_field);
    let take = fields.iter().copied().map(take_field);

    Ok(Shape {
        read: quote!({ #(#read,)* }),
        twin: quote!({ #(#hold,)* }),
        walk: quote! {
            #(#judge)*

            ::std::option::Option::Some(#parsed { #(#take,)* })
        },
        defaults,
    })
}

/// `#[validate]` writes `#[serde(default)]` and `#[schema(…)]` onto every field, and both are
/// helper attributes of a derive the caller has to supply. Without them the compiler complains
/// about an attribute the caller never wrote, so say what is actually missing.
fn derived(input: &DeriveInput) -> Vec<String> {
    input
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("derive"))
        .filter_map(|attribute| {
            attribute
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .ok()
        })
        .flatten()
        .filter_map(|path| path.segments.last().map(|last| last.ident.to_string()))
        .collect()
}

fn required_derives(input: &DeriveInput) -> syn::Result<()> {
    let asked = derived(input);

    let missing: Vec<&str> = ["Deserialize", "ToSchema"]
        .into_iter()
        .filter(|needed| !asked.iter().any(|asked| asked == needed))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    Err(syn::Error::new_spanned(
        input,
        format!(
            "#[validate] needs the type to derive {}",
            missing.join(" and ")
        ),
    ))
}

fn variants(
    body: &syn::DataEnum,
    owner: &Ident,
    parsed: &Ident,
    params: bool,
) -> syn::Result<Shape> {
    let mut read = Vec::new();
    let mut defaults = Vec::new();
    let mut hold = Vec::new();
    let mut arms = Vec::new();

    for variant in &body.variants {
        let name = &variant.ident;
        let attributes: Vec<_> = variant.attrs.iter().collect();

        // serde never reads a skipped variant, so there is nothing to judge: the twin holds it as
        // it is written.
        if skipped(variant) {
            let (twin_fields, pattern) = match &variant.fields {
                Fields::Unit => (quote!(), quote!()),
                Fields::Unnamed(unnamed) => {
                    let types = unnamed.unnamed.iter().map(|field| &field.ty);
                    let binds =
                        (0..unnamed.unnamed.len()).map(|index| format_ident!("field{index}"));

                    (quote!((#(#types,)*)), quote!((#(#binds,)*)))
                }
                Fields::Named(named) => {
                    let idents: Vec<_> = named.named.iter().map(|field| &field.ident).collect();
                    let types = named.named.iter().map(|field| &field.ty);

                    (quote!({ #(#idents: #types,)* }), quote!({ #(#idents,)* }))
                }
            };
            let fields = &variant.fields;

            read.push(quote!(#(#attributes)* #name #fields));
            hold.push(quote!(#name #twin_fields));
            arms.push(quote! {
                Self::#name #pattern => ::std::option::Option::Some(#parsed::#name #pattern),
            });

            continue;
        }

        if matches!(variant.fields, Fields::Unit) {
            read.push(quote!(#(#attributes)* #name));
            hold.push(quote!(#name));
            arms.push(quote! {
                Self::#name => ::std::option::Option::Some(#parsed::#name),
            });

            continue;
        }

        let fields = named(&variant.fields, variant)?;
        let bound: Vec<_> = fields.iter().map(|field| field.ident).collect();
        let variant_read = fields
            .iter()
            .copied()
            .map(|field| read_field(field, params, owner, Some(name)));
        defaults.extend(
            fields
                .iter()
                .copied()
                .filter_map(|field| default_function(field, Some(name))),
        );
        let variant_hold = fields.iter().copied().map(hold_field);
        let judge = fields.iter().copied().map(judge_bound_field);
        let take = fields.iter().copied().map(take_field);

        read.push(quote!(#(#attributes)* #name { #(#variant_read,)* }));
        hold.push(quote!(#name { #(#variant_hold,)* }));
        arms.push(quote! {
            Self::#name { #(#bound,)* } => {
                #(#judge)*

                ::std::option::Option::Some(#parsed::#name { #(#take,)* })
            }
        });
    }

    Ok(Shape {
        read: quote!({ #(#read,)* }),
        twin: quote!({ #(#hold,)* }),
        walk: quote!(match self { #(#arms)* }),
        defaults,
    })
}

/// `#[serde(skip)]` on a variant.
fn skipped(variant: &syn::Variant) -> bool {
    variant
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
        .filter_map(|attribute| attribute.parse_args::<Ident>().ok())
        .any(|argument| argument == "skip")
}

/// A field of a struct or a variant with named fields, so its name is always there.
#[derive(Clone, Copy)]
struct NamedField<'a> {
    ident: &'a Ident,
    field: &'a Field,
}

fn named<'a>(fields: &'a Fields, at: &dyn quote::ToTokens) -> syn::Result<Vec<NamedField<'a>>> {
    let Fields::Named(named) = fields else {
        return Err(syn::Error::new_spanned(
            at,
            "#[validate] needs named fields",
        ));
    };

    named
        .named
        .iter()
        .map(|field| {
            field
                .ident
                .as_ref()
                .map(|ident| NamedField { ident, field })
                .ok_or_else(|| syn::Error::new_spanned(field, "#[validate] needs named fields"))
        })
        .collect()
}

fn take_field(field: NamedField) -> proc_macro2::TokenStream {
    let ident = field.ident;

    quote!(#ident: #ident?)
}

/// A variant's fields arrive already bound by the `match`, so there is no `self.` to read them
/// through.
fn judge_bound_field(field: NamedField) -> proc_macro2::TokenStream {
    let ident = field.ident;

    judge(field, &quote!(#ident))
}

fn judge(field: NamedField, value: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let ident = field.ident;
    let name = wire_name(field);

    match field_default(field.field) {
        FieldDefault::Value(default) => {
            quote!(let #ident = errors.defaulted_to(#name, #value, || #default);)
        }
        FieldDefault::OfType => quote!(let #ident = errors.defaulted(#name, #value);),
        FieldDefault::None => {
            let method = match spelling(&field.field.ty).kind {
                Kind::Required => format_ident!("required"),
                Kind::Optional => format_ident!("optional"),
                Kind::Patch => format_ident!("patch"),
            };

            quote!(let #ident = errors.#method(#name, #value);)
        }
    }
}

/// The name of the field as it is sent, which is the name its errors are filed under.
fn wire_name(field: NamedField) -> String {
    let mut name = field.ident.to_string();

    for attribute in field
        .field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
    {
        let _ = attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") && meta.input.peek(syn::Token![=]) {
                name = meta.value()?.parse::<syn::LitStr>()?.value();
            } else if meta.input.peek(syn::Token![=]) {
                meta.value()?.parse::<syn::Expr>()?;
            } else if meta.input.peek(syn::token::Paren) {
                meta.input.parse::<proc_macro2::Group>()?;
            }

            Ok(())
        });
    }

    name
}

/// What an omitted field becomes, if it may be omitted at all. Neither an `Option` nor the
/// default itself appears where the value is used.
enum FieldDefault {
    None,
    /// `#[validate(default)]`: the field's own type says.
    OfType,
    /// `#[validate(default = value)]`: this value, for this field only.
    Value(syn::Expr),
}

fn field_default(field: &Field) -> FieldDefault {
    for attribute in field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("validate"))
    {
        if attribute
            .parse_args::<Ident>()
            .is_ok_and(|argument| argument == "default")
        {
            return FieldDefault::OfType;
        }

        if let Ok(syn::MetaNameValue { path, value, .. }) =
            attribute.parse_args::<syn::MetaNameValue>()
            && path.is_ident("default")
        {
            return FieldDefault::Value(value);
        }
    }

    FieldDefault::None
}

/// The field as it is read: always a `Patch`, so an absent one is `Absent` rather than an error
/// serde stops at. `value_type` and `required` put back what that costs in the document.
/// The function that the document calls for the sent form of a field's own default.
fn default_function_name(field: NamedField, variant: Option<&Ident>) -> Ident {
    let ident = field.ident;

    match variant {
        Some(variant) => format_ident!("validate_default_of_{variant}_{ident}"),
        None => format_ident!("validate_default_of_{ident}"),
    }
}

fn default_function(
    field: NamedField,
    variant: Option<&Ident>,
) -> Option<proc_macro2::TokenStream> {
    let FieldDefault::Value(default) = field_default(field.field) else {
        return None;
    };
    let function = default_function_name(field, variant);
    let ty = &field.field.ty;

    Some(quote! {
        #[doc(hidden)]
        fn #function() -> #ty {
            <#ty as ::std::convert::From<_>>::from(#default)
        }
    })
}

fn read_field(
    field: NamedField,
    params: bool,
    owner: &Ident,
    variant: Option<&Ident>,
) -> proc_macro2::TokenStream {
    let ident = field.ident;
    let Spelling {
        kind,
        inner,
        primitive,
        wrapper,
    } = spelling(&field.field.ty);
    let carried: Vec<_> = field
        .field
        .attrs
        .iter()
        .filter(|attribute| {
            !["schema", "param", "validate"]
                .iter()
                .any(|rewritten| attribute.path().is_ident(rewritten))
        })
        .collect();
    let mut extra = arguments_of(field.field, "schema");

    if primitive {
        extra.extend(quote!(inline,));
    }

    // Both optional spellings admit `null`; they differ in what it means, which a schema cannot say.
    let held = carried_type(kind, &inner);
    let wrapper = wrapper.map_or_else(
        || quote!(::serde_utoipa_validate::Patch),
        |path| quote!(#path),
    );
    let requirement = match (field_default(field.field), kind) {
        (FieldDefault::Value(_), _) => {
            let function = default_function_name(field, variant);

            quote!(required = false, default = #owner::#function,)
        }
        (FieldDefault::OfType, _) => quote!(required = false,),
        (FieldDefault::None, Kind::Required) => quote!(required = true,),
        (FieldDefault::None, _) => quote!(),
    };

    // A parameter states its schema where it stands: utoipa would otherwise point at a component
    // that nothing declares, because only a body or a response registers one.
    let parameter = params.then(|| {
        let extra = arguments_of(field.field, "param");

        quote!(#[param(value_type = #held, inline, #requirement #extra)])
    });

    quote! {
        #(#carried)*
        #[serde(default)]
        #[schema(value_type = #held, #requirement #extra)]
        #parameter
        #ident: #wrapper<#held>
    }
}

/// What sits inside the `Patch`. `Option<T>` keeps serde's meaning, so its `null` is carried by an
/// `Option` and read as absent; `Patch<T>` carries whatever was written inside it.
fn carried_type(kind: Kind, inner: &Type) -> proc_macro2::TokenStream {
    match kind {
        Kind::Required | Kind::Patch => quote!(#inner),
        Kind::Optional => quote!(::std::option::Option<#inner>),
    }
}

/// What the field becomes once judged.
fn hold_field(field: NamedField) -> proc_macro2::TokenStream {
    let ident = field.ident;
    let Spelling { kind, inner, .. } = spelling(&field.field.ty);
    let visibility = &field.field.vis;
    let carried = carried_type(kind, &inner);
    let parsed = quote!(<#carried as ::serde_utoipa_validate::Wire>::Parsed);

    // `Option<Option<_>>` rather than a type of ours: the application layer holds this, and a
    // nested `Option` needs no vocabulary shared across the layers.
    let held = match kind {
        Kind::Required | Kind::Optional => parsed,
        Kind::Patch => quote!(::std::option::Option<#parsed>),
    };

    quote!(#visibility #ident: #held)
}

fn judge_field(field: NamedField) -> proc_macro2::TokenStream {
    let ident = field.ident;

    judge(field, &quote!(self.#ident))
}

/// Anything the caller put in `#[schema(...)]` or `#[param(...)]`, kept so `inline` and friends
/// still work. The attribute itself is rewritten, so what was inside it has to be carried over.
fn arguments_of(field: &Field, attribute: &str) -> proc_macro2::TokenStream {
    field
        .attrs
        .iter()
        .filter(|carried| carried.path().is_ident(attribute))
        .filter_map(|attribute| attribute.parse_args::<proc_macro2::TokenStream>().ok())
        .collect()
}

/// A field written as a primitive is read through the wrapper that carries a wrong JSON type as a
/// rejection rather than letting serde end the body. Nothing about that is the caller's business,
/// so it happens here and the schema is inlined: the reader's name means nothing to a client.
fn wrapped(ty: &Type) -> Option<Type> {
    let Type::Path(path) = ty else {
        return None;
    };

    let segment = path.path.segments.last()?;

    if segment.ident == "Option" {
        let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
            return None;
        };
        let inner = wrapped(inner)?;

        return Some(syn::parse_quote!(::std::option::Option<#inner>));
    }

    match segment.ident.to_string().as_str() {
        "String" => Some(syn::parse_quote!(::serde_utoipa_validate::RawString)),
        "u64" => Some(syn::parse_quote!(::serde_utoipa_validate::RawNumber)),
        "Uuid" => Some(syn::parse_quote!(::serde_utoipa_validate::RawUuid)),
        "Timestamp" => Some(syn::parse_quote!(::serde_utoipa_validate::RawTimestamp)),
        _ => None,
    }
}

/// What the caller wrote, and the wire type underneath it.
#[derive(Clone, Copy)]
enum Kind {
    Required,
    Optional,
    Patch,
}

/// How a field was spelled, once the wrapper and any primitive have been resolved.
struct Spelling {
    kind: Kind,
    /// What sits under the wrapper.
    inner: Type,
    /// The caller wrote a primitive and a reader was substituted, so the schema is inlined.
    primitive: bool,
    /// `Patch` as the caller wrote it, so the field goes on naming it and their import stays used.
    wrapper: Option<syn::Path>,
}

fn spelling(ty: &Type) -> Spelling {
    fn under(ty: &Type) -> (Type, bool) {
        match wrapped(ty) {
            Some(substituted) => (substituted, true),
            None => (ty.clone(), false),
        }
    }

    let Type::Path(path) = ty else {
        let (inner, primitive) = under(ty);

        return Spelling {
            kind: Kind::Required,
            inner,
            primitive,
            wrapper: None,
        };
    };

    let written = path.path.segments.last().and_then(|segment| {
        let kind = match segment.ident.to_string().as_str() {
            "Option" => Kind::Optional,
            "Patch" => Kind::Patch,
            _ => return None,
        };
        let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
            return None;
        };

        Some((kind, inner))
    });

    // Only `Patch` is written by a caller, so only it has a path worth keeping.
    let wrapper = matches!(written, Some((Kind::Patch, _))).then(|| {
        let mut written = path.path.clone();

        if let Some(last) = written.segments.last_mut() {
            last.arguments = syn::PathArguments::None;
        }

        written
    });

    let (kind, written) = written.unwrap_or((Kind::Required, ty));
    let (inner, primitive) = under(written);

    Spelling {
        kind,
        inner,
        primitive,
        wrapper,
    }
}
