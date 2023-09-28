mod item_fn;

mod main_fn;

mod test_fn;

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn main(_args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as main_fn::CompioMain)
        .into_token_stream()
        .into()
}

#[proc_macro_attribute]
pub fn test(_args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as test_fn::CompioTest)
        .into_token_stream()
        .into()
}
