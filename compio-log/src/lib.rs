#[cfg_attr(not(feature = "enable_log"), doc(hidden))]
pub use tracing::*;

#[cfg(not(feature = "enable_log"))]
pub mod dummy;

#[cfg(feature = "enable_log")]
#[macro_export]
macro_rules! instrument {
    ($lvl:expr, $name:expr, $($fields:tt)*) => {
        let _guard = $crate::span!(target:module_path!(), $lvl, $name, $($fields)*).entered();
    };
    ($lvl:expr, $name:expr) => {
        let _guard = $crate::span!(target:module_path!(), $lvl, $name).entered();
    };
}

#[cfg(not(feature = "enable_log"))]
#[macro_export]
macro_rules! instrument {
    ($lvl:expr, $name:expr, $($fields:tt)*) => {};
    ($lvl:expr, $name:expr) => {};
}
