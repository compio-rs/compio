#[macro_export]
macro_rules! event {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::event!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! error {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::error!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::warn!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! info {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::info!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! debug {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::debug!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! trace {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::trace!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! span {
    ($($args:tt)*) => {{
        if false {
            $crate::__tracing::span!($($args)*)
        } else {
            $crate::Span::none()
        }
    }};
}

#[macro_export]
macro_rules! error_span {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::error_span!($($args)*)
        } else {
            $crate::Span::none()
        }
    };
}

#[macro_export]
macro_rules! warn_span {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::warn_span!($($args)*)
        } else {
            $crate::Span::none()
        }
    };
}

#[macro_export]
macro_rules! info_span {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::info_span!($($args)*)
        } else {
            $crate::Span::none()
        }
    };
}

#[macro_export]
macro_rules! debug_span {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::debug_span!($($args)*)
        } else {
            $crate::Span::none()
        }
    };
}

#[macro_export]
macro_rules! trace_span {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::trace_span!($($args)*)
        } else {
            $crate::Span::none()
        }
    };
}
