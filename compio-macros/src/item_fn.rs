use proc_macro2::TokenStream;
use quote::{ToTokens, TokenStreamExt, quote};
use syn::{
    Attribute, Expr, ExprLit, Ident, Lit, Meta, Path, Signature, Token, Visibility, parse::Parse,
    punctuated::Punctuated,
};

use crate::{retrieve_driver_mod, retrieve_runtime_mod};

struct MetaPunctuated(Punctuated<Meta, Token![,]>);

impl Parse for MetaPunctuated {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self(Punctuated::parse_terminated(input)?))
    }
}

pub(crate) struct BuilderMethod {
    pub name: Ident,
    pub value: Expr,
}

#[derive(Default)]
pub(crate) struct RawAttr {
    pub runtime_methods: Vec<BuilderMethod>,
    pub proactor_methods: Vec<BuilderMethod>,
    pub crate_name: Option<TokenStream>,
    pub with_proactor_call: Option<Path>,
}

impl Parse for RawAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let items = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        let mut runtime_methods = Vec::new();
        let mut proactor_methods = Vec::new();
        let mut crate_name = None;
        let mut with_proactor_call = None;

        for meta in items {
            match meta {
                Meta::List(list) => {
                    if list.path.is_ident("with_proactor") {
                        if !list.tokens.is_empty() {
                            let inner_items = syn::parse2::<MetaPunctuated>(list.tokens)?.0;
                            for inner_meta in inner_items {
                                if let Meta::NameValue(nv) = inner_meta {
                                    let name = nv.path.require_ident()?.clone();
                                    proactor_methods.push(BuilderMethod {
                                        name,
                                        value: nv.value,
                                    });
                                } else {
                                    return Err(syn::Error::new_spanned(
                                        inner_meta,
                                        "expected `name = value` inside `with_proactor`",
                                    ));
                                }
                            }
                            with_proactor_call = Some(list.path.clone());
                        }
                    } else {
                        return Err(syn::Error::new_spanned(
                            list.path,
                            "unknown key; use `name = value` for parameters or \
                             `with_proactor(...)` for proactor config",
                        ));
                    }
                }
                Meta::NameValue(nv) => {
                    if nv.path.is_ident("crate") {
                        if let Expr::Lit(ExprLit {
                            lit: Lit::Str(s), ..
                        }) = &nv.value
                        {
                            crate_name = Some(s.parse::<TokenStream>()?);
                        } else {
                            crate_name = Some(nv.value.into_token_stream());
                        }
                    } else {
                        let name = nv.path.require_ident()?.clone();
                        runtime_methods.push(BuilderMethod {
                            name,
                            value: nv.value,
                        });
                    }
                }
                Meta::Path(path) => {
                    return Err(syn::Error::new_spanned(
                        path,
                        "expected `name = value` or `with_proactor(...)`",
                    ));
                }
            }
        }

        Ok(Self {
            runtime_methods,
            proactor_methods,
            crate_name,
            with_proactor_call,
        })
    }
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
            Some(c) => {
                let c = c.clone();
                quote!(#c::runtime)
            }
            None => retrieve_runtime_mod(),
        };

        let driver_mod = match &self.args.crate_name {
            Some(c) => {
                let c = c.clone();
                quote!(#c::driver)
            }
            None => retrieve_driver_mod(),
        };

        let block = &self.body;

        let mut builder = quote! {
            #runtime_mod::Runtime::builder()
        };

        for method in &self.args.runtime_methods {
            let name = &method.name;
            let value = &method.value;
            builder = quote! {
                #builder.#name(#value)
            };
        }

        if !self.args.proactor_methods.is_empty() {
            let mut proactor_stmts: Vec<TokenStream> = Vec::new();
            proactor_stmts.push(quote! {
                let mut __compio_proactor_builder = #driver_mod::Proactor::builder();
            });
            for method in &self.args.proactor_methods {
                let name = &method.name;
                let value = &method.value;
                proactor_stmts.push(quote! {
                    __compio_proactor_builder.#name(#value);
                });
            }
            // Preserve the original token for the `with_proactor` call to make the language
            // server work better.
            let with_proactor_call = if let Some(path) = &self.args.with_proactor_call {
                quote!(#path)
            } else {
                quote!(with_proactor)
            };
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
