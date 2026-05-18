#[macro_export]
macro_rules! debug {
    ($($args:tt)*) => {
        $crate::event!($crate::Level::DEBUG, $($args)*);
    };
}

#[macro_export]
macro_rules! debug_span {
    ($($args:tt)*) => {
        $crate::span!($crate::Level::DEBUG, $($args)*)
    };
}

#[macro_export]
macro_rules! error {
    ($($args:tt)*) => {
        $crate::event!($crate::Level::ERROR, $($args)*);
    };
}

#[macro_export]
macro_rules! error_span {
    ($($args:tt)*) => {
        $crate::span!($crate::Level::ERROR, $($args)*)
    };
}

#[macro_export]
macro_rules! event {
    ($($args:tt)*) => {
        if false {
            $crate::__tracing::event!($($args)*);
        }
    };
}

#[macro_export]
macro_rules! info {
    ($($args:tt)*) => {
        $crate::event!($crate::Level::INFO, $($args)*);
    };
}

#[macro_export]
macro_rules! info_span {
    ($($args:tt)*) => {
        $crate::span!($crate::Level::INFO, $($args)*)
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
macro_rules! trace {
    ($($args:tt)*) => {
        $crate::event!($crate::Level::TRACE, $($args)*);
    };
}

#[macro_export]
macro_rules! trace_span {
    ($($args:tt)*) => {
        $crate::span!($crate::Level::TRACE, $($args)*)
    };
}

#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => {
        $crate::event!($crate::Level::WARN, $($args)*);
    };
}

#[macro_export]
macro_rules! warn_span {
    ($($args:tt)*) => {
        $crate::span!($crate::Level::WARN, $($args)*)
    };
}
