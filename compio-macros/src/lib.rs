mod item_fn;

mod main_fn;
use main_fn::CompioMain;

mod test_fn;
use test_fn::CompioTest;

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse_macro_input;

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
