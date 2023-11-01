mod item_fn;

mod main_fn;

mod test_fn;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{quote, ToTokens};
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn main(args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as main_fn::CompioMain)
        .with_args(parse_macro_input!(args as item_fn::RawAttr))
        .into_token_stream()
        .into()
}

#[proc_macro_attribute]
pub fn test(args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as test_fn::CompioTest)
        .with_args(parse_macro_input!(args as item_fn::RawAttr))
        .into_token_stream()
        .into()
}

fn retrieve_runtime_mod() -> proc_macro2::TokenStream {
    match crate_name("compio-runtime") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
        Err(_) => match crate_name("compio") {
            Ok(FoundCrate::Itself) => quote!(crate::runtime),
            Ok(FoundCrate::Name(name)) => {
                let ident = Ident::new(&name, Span::call_site());
                quote!(::#ident::runtime)
            }
            Err(_) => panic!("Cannot find compio or compio_runtime."),
        },
    }
}
