//! The compio runtime.
//!
//! ```
//! let ans = compio_runtime::Runtime::new().unwrap().block_on(async {
//!     println!("Hello world!");
//!     42
//! });
//! assert_eq!(ans, 42);
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "current_thread_id", feature(current_thread_id))]
#![cfg_attr(feature = "future-combinator", feature(context_ext, local_waker))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

mod affinity;
mod attacher;
pub mod fd;
mod runtime;

#[cfg(feature = "event")]
pub mod event;
#[cfg(feature = "future-combinator")]
pub mod future;
#[cfg(feature = "time")]
pub mod time;

pub use async_task::Task;
pub use attacher::*;
use compio_buf::BufResult;
#[allow(hidden_glob_reexports, unused)]
use runtime::RuntimeInner; // used to shadow glob export so that RuntimeInner is not exported
pub use runtime::*;
/// Macro that asserts a type *DOES NOT* implement some trait. Shamelessly
/// copied from <https://users.rust-lang.org/t/a-macro-to-assert-that-a-type-does-not-implement-trait-bounds/31179>.
///
/// # Example
///
/// ```rust,ignore
/// assert_not_impl!(u8, From<u16>);
/// ```
macro_rules! assert_not_impl {
    ($x:ty, $($t:path),+ $(,)*) => {
        const _: fn() -> () = || {
            struct Check<T: ?Sized>(T);
            trait AmbiguousIfImpl<A> { fn some_item() { } }

            impl<T: ?Sized> AmbiguousIfImpl<()> for Check<T> { }
            impl<T: ?Sized $(+ $t)*> AmbiguousIfImpl<u8> for Check<T> { }

            <Check::<$x> as AmbiguousIfImpl<_>>::some_item()
        };
    };
}

use assert_not_impl;
