use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::{ItemFn, parse::Parse};

use crate::item_fn::{RawAttr, RawBodyItemFn};

pub(crate) struct CompioMain(pub RawBodyItemFn);

impl CompioMain {
    pub fn with_args(mut self, args: RawAttr) -> Self {
        self.0.set_args(args);
        self
    }

    pub fn with_test(mut self, test: bool) -> Self {
        self.0.set_test(test);
        self
    }
}

impl Parse for CompioMain {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut item_fn = input.parse::<ItemFn>()?;

        if item_fn.sig.asyncness.is_none() {
            return Err(syn::Error::new_spanned(
                item_fn.sig.ident,
                "the `async` keyword is missing from the function declaration",
            ));
        };

        item_fn.sig.asyncness.take();
        Ok(Self(RawBodyItemFn::new(item_fn)))
    }
}

impl ToTokens for CompioMain {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.emit_fn_to_tokens(tokens);
    }
}
