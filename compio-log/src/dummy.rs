#[macro_export]
macro_rules! debug {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! debug_span {
    ($($args:tt)*) => {
        $crate::Span::none()
    };
}

#[macro_export]
macro_rules! error {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! error_span {
    ($($args:tt)*) => {
        $crate::Span::none()
    };
}

#[macro_export]
macro_rules! event {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! info {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! info_span {
    ($($args:tt)*) => {
        $crate::Span::none()
    };
}

#[macro_export]
macro_rules! span {
    ($($args:tt)*) => {
        $crate::Span::none()
    };
}

#[macro_export]
macro_rules! trace {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! trace_span {
    ($($args:tt)*) => {
        $crate::Span::none()
    };
}

#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => {};
}

#[macro_export]
macro_rules! warn_span {
    ($($args:tt)*) => {
        $crate::Span::none()
    };
}
