use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{parse::Parse, parse_macro_input, AttrStyle, Attribute, Signature, Visibility};

struct CompioMain {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
    pub block: TokenStream,
}

impl Parse for CompioMain {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let outer_attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let mut sig: Signature = input.parse()?;
        let block: TokenStream = input.parse()?;

        if let None = sig.asyncness {
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
        Ok(Self {
            attrs: outer_attrs,
            vis,
            sig,
            block,
        })
    }
}

impl ToTokens for CompioMain {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(
            self.attrs
                .iter()
                .filter(|a| matches!(a.style, AttrStyle::Outer)),
        );
        self.vis.to_tokens(tokens);
        self.sig.to_tokens(tokens);
        let body = &self.block;
        tokens.append_all(quote!({
            compio::task::block_on(async move #body)
        }));
    }
}

#[proc_macro_attribute]
pub fn main(
    _args: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    parse_macro_input!(item as CompioMain)
        .into_token_stream()
        .into()
}