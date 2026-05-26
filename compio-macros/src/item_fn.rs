use std::{collections::HashMap, ops::Deref};

use darling::{FromMeta, util::parse_expr::parse_str_literal};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, TokenStreamExt, quote};
use syn::{Attribute, Expr, Ident, Meta, Path, Signature, Visibility, spanned::Spanned};

use crate::{retrieve_driver_mod, retrieve_runtime_mod};

fn parse_str_literal_optional(meta: &Meta) -> darling::Result<Option<Expr>> {
    Ok(Some(parse_str_literal(meta)?))
}

#[derive(Debug)]
struct KeepPathSpan<T> {
    span: Span,
    value: T,
}

impl<T: Default> Default for KeepPathSpan<T> {
    fn default() -> Self {
        Self {
            span: Span::call_site(),
            value: T::default(),
        }
    }
}

impl<T: FromMeta> FromMeta for KeepPathSpan<T> {
    fn from_meta(meta: &Meta) -> darling::Result<Self> {
        let path = meta.path();
        Ok(Self {
            span: path.span(),
            value: T::from_meta(meta)?,
        })
    }
}

impl<T> Deref for KeepPathSpan<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

#[derive(Default, FromMeta)]
#[darling(derive_syn_parse)]
pub struct RawAttr {
    #[darling(default, with = parse_str_literal_optional, rename = "crate")]
    crate_name: Option<Expr>,
    #[darling(default, flatten)]
    runtime_methods: HashMap<Path, Expr>,
    #[darling(default)]
    with_proactor: KeepPathSpan<HashMap<Path, Expr>>,
}

pub(crate) struct RawBodyItemFn {
    pub attrs: Vec<Attribute>,
    pub args: RawAttr,
    pub vis: Visibility,
    pub sig: Signature,
    pub body: TokenStream,
    pub test: bool,
}

impl RawBodyItemFn {
    pub fn new(attrs: Vec<Attribute>, vis: Visibility, sig: Signature, body: TokenStream) -> Self {
        Self {
            attrs,
            args: RawAttr::default(),
            vis,
            sig,
            body,
            test: false,
        }
    }

    pub fn set_args(&mut self, args: RawAttr) {
        self.args = args;
    }

    pub fn set_test(&mut self, test: bool) {
        self.test = test;
    }

    pub fn emit_fn_to_tokens(&self, tokens: &mut TokenStream) {
        if self.test {
            tokens.append_all(quote!(#[test]));
        }
        tokens.append_all(
            self.attrs
                .iter()
                .filter(|a| matches!(a.style, syn::AttrStyle::Outer)),
        );
        self.vis.to_tokens(tokens);
        self.sig.to_tokens(tokens);
        tokens.append_all(self.gen_runtime_block());
    }

    fn gen_runtime_block(&self) -> TokenStream {
        let runtime_mod = match &self.args.crate_name {
            Some(c) => quote!(#c::runtime),
            None => retrieve_runtime_mod(),
        };

        let driver_mod = match &self.args.crate_name {
            Some(c) => quote!(#c::driver),
            None => retrieve_driver_mod(),
        };

        let block = &self.body;

        let mut builder = quote! {
            #runtime_mod::Runtime::builder()
        };

        for (name, value) in &self.args.runtime_methods {
            builder = quote! {
                #builder.#name(#value)
            };
        }

        if !self.args.with_proactor.is_empty() {
            let mut proactor_stmts: Vec<TokenStream> = Vec::new();
            proactor_stmts.push(quote! {
                let mut __compio_proactor_builder = #driver_mod::Proactor::builder();
            });
            for (name, value) in self.args.with_proactor.iter() {
                proactor_stmts.push(quote! {
                    __compio_proactor_builder.#name(#value);
                });
            }

            let with_proactor_call = Ident::new("with_proactor", self.args.with_proactor.span);

            builder = quote! {
                #builder.#with_proactor_call({
                    #(#proactor_stmts)*
                    __compio_proactor_builder
                })
            };
        }

        quote!({
            #builder.build().expect("cannot create runtime").block_on(async move #block)
        })
    }
}
