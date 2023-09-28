mod main_fn;
use main_fn::CompioMain;

use quote::ToTokens;
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn main(
    _args: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    parse_macro_input!(item as CompioMain)
        .into_token_stream()
        .into()
}
