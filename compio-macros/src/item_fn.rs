use std::{collections::HashMap, ops::Deref};

use darling::{
    FromMeta,
    util::{Flag, parse_expr::parse_str_literal},
};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, TokenStreamExt, quote};
use syn::{Expr, Ident, ItemFn, Meta, Path, parse::Parse, spanned::Spanned};

use crate::{retrieve_console_subscriber_mod, retrieve_driver_mod, retrieve_runtime_mod};

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
    console: Flag,
    #[darling(default, flatten)]
    runtime_methods: HashMap<Path, Expr>,
    #[darling(default)]
    with_proactor: KeepPathSpan<HashMap<Path, Expr>>,
}

/// The arguments of `#[compio::main]`.
pub(crate) type MainAttr = Attr<false>;

/// The arguments of `#[compio::test]`, which are the arguments of
/// `#[compio::main]` less `console`.
pub(crate) type TestAttr = Attr<true>;

/// The arguments of one of the two attributes, parsed as the attribute that
/// takes them.
///
/// Which of the two it is is `TEST`, so the type that parsed the arguments is
/// also what says whether a `#[test]` is to be emitted for them, and the caller
/// is not asked to say it a second time.
pub(crate) struct Attr<const TEST: bool>(RawAttr);

impl<const TEST: bool> Parse for Attr<TEST> {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let attr = input.parse::<RawAttr>()?;

        if TEST && attr.console.is_present() {
            return Err(syn::Error::new(
                attr.console.span(),
                "`console` is only supported on `#[compio::main]`, since the subscriber it \
                 installs is the process-wide default and a test binary runs more than one test",
            ));
        }

        Ok(Self(attr))
    }
}

pub(crate) struct RawBodyItemFn {
    pub args: RawAttr,
    pub item_fn: ItemFn,
    test: bool,
}

impl RawBodyItemFn {
    pub fn new(item_fn: ItemFn) -> Self {
        Self {
            args: RawAttr::default(),
            item_fn,
            test: false,
        }
    }

    pub fn set_args<const TEST: bool>(&mut self, Attr(args): Attr<TEST>) {
        self.args = args;
        self.test = TEST;
    }

    pub fn emit_fn_to_tokens(&self, tokens: &mut TokenStream) {
        if self.test {
            tokens.append_all(quote!(#[test]));
        }
        tokens.append_all(self.item_fn.attrs.iter());
        self.item_fn.vis.to_tokens(tokens);
        self.item_fn.sig.to_tokens(tokens);
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

        let console_init = self.gen_console_init();

        let block = &self.item_fn.block;
        quote!({
            #console_init
            #builder.build().expect("cannot create runtime").block_on(async move #block)
        })
    }

    /// The `console_subscriber::init()` call the `console` argument asks for.
    ///
    /// This is what the argument is for: a subscriber installed by the body
    /// itself is installed *inside* `block_on`, too late for the span of the
    /// task that runs the body, which is created on the way in and stays
    /// disabled for its whole life. Installing it out here, around the
    /// `block_on` rather than within it, is the one thing the body cannot do
    /// for itself.
    fn gen_console_init(&self) -> TokenStream {
        if !self.args.console.is_present() {
            return quote!();
        }

        // Resolved rather than spelled out, so that a renamed dependency works.
        // The manifest is all this can read, and it reads every dependency
        // table: a `console-subscriber` that is there but not linked --
        // dev-only for a build that is not a test, or optional and off
        // -- is found here and left to rustc, which is the one that
        // knows.
        match retrieve_console_subscriber_mod() {
            Some(console_mod) => quote!(#console_mod::init();),
            None => syn::Error::new(
                self.args.console.span(),
                "`console` needs a dependency on `console-subscriber`, which is what installs the \
                 subscriber that reports to the console",
            )
            .to_compile_error(),
        }
    }
}
