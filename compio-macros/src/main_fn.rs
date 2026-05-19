use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::{Attribute, Signature, Visibility, parse::Parse};

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
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let mut sig: Signature = input.parse()?;
        let body: TokenStream = input.parse()?;

        if sig.asyncness.is_none() {
            return Err(syn::Error::new_spanned(
                sig.ident,
                "the `async` keyword is missing from the function declaration",
            ));
        };

        sig.asyncness.take();
        Ok(Self(RawBodyItemFn::new(attrs, vis, sig, body)))
    }
}

impl ToTokens for CompioMain {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.emit_fn_to_tokens(tokens);
    }
}
