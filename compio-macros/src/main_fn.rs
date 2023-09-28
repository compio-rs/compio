use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{parse::Parse, AttrStyle, Attribute, Signature, Visibility};

use crate::item_fn::RawBodyItemFn;

pub(crate) struct CompioMain(pub RawBodyItemFn);

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
        if sig.ident != Ident::new("main", Span::call_site()) {
            return Err(syn::Error::new_spanned(
                sig.ident,
                "`compio::main` can only be used for main function.",
            ));
        }

        sig.asyncness.take();
        Ok(Self(RawBodyItemFn::new(attrs, vis, sig, body)))
    }
}

impl ToTokens for CompioMain {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(
            self.0
                .attrs
                .iter()
                .filter(|a| matches!(a.style, AttrStyle::Outer)),
        );
        self.0.vis.to_tokens(tokens);
        self.0.sig.to_tokens(tokens);
        let block = &self.0.body;
        tokens.append_all(quote!({
            compio::task::block_on(async move #block)
        }));
    }
}
