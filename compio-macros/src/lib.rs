mod main_fn;
use main_fn::CompioMain;

mod test_fn;
use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse_macro_input;
use test_fn::CompioTest;

#[proc_macro_attribute]
pub fn main(_args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as CompioMain)
        .into_token_stream()
        .into()
}

#[proc_macro_attribute]
pub fn test(_args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as CompioTest)
        .into_token_stream()
        .into()
}
