use proc_macro2::TokenStream;
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{parse::Parse, AttrStyle, Attribute, Signature, Visibility};

pub(crate) struct CompioTest {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
    pub body: TokenStream,
}

impl Parse for CompioTest {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let outer_attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let mut sig: Signature = input.parse()?;
        let body: TokenStream = input.parse()?;

        if let None = sig.asyncness {
            return Err(syn::Error::new_spanned(
                sig.ident,
                "the `async` keyword is missing from the function declaration",
            ));
        };
        sig.asyncness.take();
        Ok(Self {
            attrs: outer_attrs,
            vis,
            sig,
            body,
        })
    }
}

impl ToTokens for CompioTest {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(quote!(#[test]));
        tokens.append_all(
            self.attrs
                .iter()
                .filter(|a| matches!(a.style, AttrStyle::Outer)),
        );
        self.vis.to_tokens(tokens);
        self.sig.to_tokens(tokens);
        let block = &self.body;
        tokens.append_all(quote!({
            compio::task::block_on(async move #block)
        }));
    }
}