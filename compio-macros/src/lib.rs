#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

mod item_fn;

mod main_fn;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span};
use quote::{ToTokens, quote};
use syn::parse_macro_input;

/// Run an `async fn main` on a runtime of its own.
///
/// The body becomes the future handed to `Runtime::block_on`, and the arguments
/// configure what it is handed to:
///
/// * `crate = path` names the crate to reach the runtime through, for a
///   `compio` that is renamed or reached through another crate;
/// * any other `name = value` becomes a `RuntimeBuilder` method call, as does
///   every `name = value` within `with_proactor(..)` on the `ProactorBuilder`;
/// * `console` installs `console_subscriber::init()` before the runtime runs.
///   The caller needs a dependency on `console-subscriber` for it to be there
///   to install, the `console_without_tokio_unstable` rustflag for the install
///   not to panic on the way up, and the `console` feature for there to be any
///   instrumentation for it to report on. Only the first of the three can be
///   checked from here; see the [`console`] module for the other two.
///
/// ```ignore
/// #[compio::main(event_interval = 8, with_proactor(capacity = 1024))]
/// async fn main() {
///     // ...
/// }
/// ```
///
/// [`console`]: https://docs.rs/compio-runtime/latest/compio_runtime/console/
#[proc_macro_attribute]
pub fn main(args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as main_fn::CompioMain)
        .with_args(parse_macro_input!(args as item_fn::MainAttr))
        .into_token_stream()
        .into()
}

/// Run an `async fn` test on a runtime of its own.
///
/// This takes the same arguments as [`main`], other than `console`: the
/// subscriber it installs is the process-wide default, which only the first
/// test of a binary could set.
#[proc_macro_attribute]
pub fn test(args: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(item as main_fn::CompioMain)
        .with_args(parse_macro_input!(args as item_fn::TestAttr))
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

/// The `console-subscriber` dependency of whoever asked for the `console`
/// argument, or [`None`] if their manifest declares none at all. A dependency
/// that is declared but not linked -- dev-only for a build that is not a test,
/// or optional and off -- is found here all the same, and left to rustc.
fn retrieve_console_subscriber_mod() -> Option<proc_macro2::TokenStream> {
    match crate_name("console-subscriber") {
        Ok(FoundCrate::Itself) => Some(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Some(quote!(::#ident))
        }
        Err(_) => None,
    }
}

fn retrieve_driver_mod() -> proc_macro2::TokenStream {
    match crate_name("compio-driver") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
        Err(_) => match crate_name("compio") {
            Ok(FoundCrate::Itself) => quote!(crate::driver),
            Ok(FoundCrate::Name(name)) => {
                let ident = Ident::new(&name, Span::call_site());
                quote!(::#ident::driver)
            }
            Err(_) => {
                let ident = Ident::new("compio_driver", Span::call_site());
                quote!(::#ident)
            }
        },
    }
}
