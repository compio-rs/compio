use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    parse::Parse, punctuated::Punctuated, Attribute, Expr, Lit, Meta, Signature, Visibility,
};

type AttributeArgs = Punctuated<syn::Meta, syn::Token![,]>;

pub(crate) struct RawAttr {
    pub inner_attrs: AttributeArgs,
}

impl Parse for RawAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let inner_attrs = AttributeArgs::parse_terminated(input)?;
        Ok(Self { inner_attrs })
    }
}

pub(crate) struct RawBodyItemFn {
    pub attrs: Vec<Attribute>,
    pub args: AttributeArgs,
    pub vis: Visibility,
    pub sig: Signature,
    pub body: TokenStream,
}

impl RawBodyItemFn {
    pub fn new(attrs: Vec<Attribute>, vis: Visibility, sig: Signature, body: TokenStream) -> Self {
        Self {
            attrs,
            args: AttributeArgs::new(),
            vis,
            sig,
            body,
        }
    }

    pub fn set_args(&mut self, args: AttributeArgs) {
        self.args = args;
    }

    pub fn crate_name(&self) -> Option<TokenStream> {
        for attr in &self.args {
            if let Meta::NameValue(name) = &attr {
                let ident = name
                    .path
                    .get_ident()
                    .map(|ident| ident.to_string().to_lowercase())
                    .unwrap_or_default();
                if ident == "crate" {
                    if let Expr::Lit(lit) = &name.value {
                        if let Lit::Str(s) = &lit.lit {
                            let crate_name = s.parse::<TokenStream>().unwrap();
                            return Some(quote!(#crate_name::runtime));
                        }
                    }
                } else {
                    panic!("Unsupported property {}", ident);
                }
            }
        }
        None
    }
}
