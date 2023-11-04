#[doc(hidden)]
pub use tracing::*;
pub use tracing_subscriber as subscriber;

#[macro_export]
macro_rules! debug {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::debug!($($args)*)
    };
}

#[macro_export]
macro_rules! debug_span {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::debug_span!($($args)*)
    };
}

#[macro_export]
macro_rules! error {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::error!($($args)*)
    };
}

#[macro_export]
macro_rules! error_span {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::error_span!($($args)*)
    };
}

#[macro_export]
macro_rules! event {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::event!($($args)*)
    };
}

#[macro_export]
macro_rules! info {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::info!($($args)*)
    };
}

#[macro_export]
macro_rules! info_span {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::info_span!($($args)*)
    };
}

#[macro_export]
macro_rules! span {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::span!($($args)*)
    };
}

#[macro_export]
macro_rules! trace {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::trace!($($args)*)
    };
}

#[macro_export]
macro_rules! trace_span {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::trace_span!($($args)*)
    };
}

#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::warn!($($args)*)
    };
}

#[macro_export]
macro_rules! warn_span {
    ($($args:tt)*) => {
        #[cfg(feature = "enable-log")]
        ::tracing::warn_span!($($args)*)
    };
}
