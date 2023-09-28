use proc_macro2::TokenStream;
use syn::{Attribute, Signature, Visibility};

pub(crate) struct RawBodyItemFn {
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub sig: Signature,
    pub body: TokenStream,
}

impl RawBodyItemFn {
    pub fn new(attrs: Vec<Attribute>, vis: Visibility, sig: Signature, body: TokenStream) -> Self {
        Self {
            attrs,
            vis,
            sig,
            body,
        }
    }
}
