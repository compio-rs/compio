use std::io;
#[cfg(feature = "try_trait_v2")]
use std::{
    convert::Infallible,
    ops::{ControlFlow, FromResidual, Residual, Try},
};

use crate::IntoInner;

/// A specialized `Result` type for operations with buffers.
///
/// This type is used as a return value for asynchronous compio methods that
/// require passing ownership of a buffer to the runtime. When the operation
/// completes, the buffer is returned no matter if the operation completed
/// successfully.
#[must_use]
pub struct BufResult<T, B>(pub io::Result<T>, pub B);

impl<T, B> BufResult<T, B> {
    /// Returns [`true`] if the result is [`Ok`].
    pub const fn is_ok(&self) -> bool {
        self.0.is_ok()
    }

    /// Returns [`true`] if the result is [`Err`].
    pub const fn is_err(&self) -> bool {
        self.0.is_err()
    }

    /// Maps the result part, and allows updating the buffer.
    #[inline]
    pub fn map<U>(self, f: impl FnOnce(T, B) -> (U, B)) -> BufResult<U, B> {
        match self.0 {
            Ok(res) => {
                let (res, buf) = f(res, self.1);
                BufResult(Ok(res), buf)
            }
            Err(e) => BufResult(Err(e), self.1),
        }
    }

    /// Maps the result part, and allows changing the buffer type.
    #[inline]
    pub fn map2<U, C>(
        self,
        f_ok: impl FnOnce(T, B) -> (U, C),
        f_err: impl FnOnce(B) -> C,
    ) -> BufResult<U, C> {
        match self.0 {
            Ok(res) => {
                let (res, buf) = f_ok(res, self.1);
                BufResult(Ok(res), buf)
            }
            Err(e) => BufResult(Err(e), f_err(self.1)),
        }
    }

    /// Maps the result part, and keeps the buffer unchanged.
    #[inline]
    pub fn map_res<U>(self, f: impl FnOnce(T) -> U) -> BufResult<U, B> {
        BufResult(self.0.map(f), self.1)
    }

    /// Maps the buffer part, and keeps the result unchanged.
    #[inline]
    pub fn map_buffer<C>(self, f: impl FnOnce(B) -> C) -> BufResult<T, C> {
        BufResult(self.0, f(self.1))
    }

    /// Returns the contained [`Ok`] value, consuming the `self` value.
    #[inline]
    pub fn expect(self, msg: &str) -> (T, B) {
        (self.0.expect(msg), self.1)
    }

    /// Returns the contained [`Ok`] value, consuming the `self` value.
    #[inline]
    pub fn unwrap(self) -> (T, B) {
        (self.0.unwrap(), self.1)
    }
}

impl<T, B> From<(io::Result<T>, B)> for BufResult<T, B> {
    fn from((res, buf): (io::Result<T>, B)) -> Self {
        Self(res, buf)
    }
}

impl<T, B> From<BufResult<T, B>> for (io::Result<T>, B) {
    fn from(BufResult(res, buf): BufResult<T, B>) -> Self {
        (res, buf)
    }
}

impl<T: IntoInner, O> IntoInner for BufResult<O, T> {
    type Inner = BufResult<O, T::Inner>;

    fn into_inner(self) -> Self::Inner {
        BufResult(self.0, self.1.into_inner())
    }
}

/// ```
/// # use compio_buf::BufResult;
/// fn foo() -> BufResult<i32, i32> {
///     let (a, b) = BufResult(Ok(1), 2)?;
///     assert_eq!(a, 1);
///     assert_eq!(b, 2);
///     (Ok(3), 4).into()
/// }
/// assert!(foo().is_ok());
/// ```
#[cfg(feature = "try_trait_v2")]
impl<T, B> FromResidual<BufResult<Infallible, B>> for BufResult<T, B> {
    fn from_residual(residual: BufResult<Infallible, B>) -> Self {
        match residual {
            BufResult(Err(e), b) => BufResult(Err(e), b),
            _ => unreachable!(),
        }
    }
}

/// ```
/// # use compio_buf::BufResult;
/// fn foo() -> std::io::Result<i32> {
///     let (a, b) = BufResult(Ok(1), 2)?;
///     assert_eq!(a, 1);
///     assert_eq!(b, 2);
///     Ok(3)
/// }
/// assert!(foo().is_ok());
/// ```
#[cfg(feature = "try_trait_v2")]
impl<T, B> FromResidual<BufResult<Infallible, B>> for io::Result<T> {
    fn from_residual(residual: BufResult<Infallible, B>) -> Self {
        match residual {
            BufResult(Err(e), _) => Err(e),
            _ => unreachable!(),
        }
    }
}

#[cfg(feature = "try_trait_v2")]
impl<T, B> Try for BufResult<T, B> {
    type Output = (T, B);
    type Residual = BufResult<Infallible, B>;

    fn from_output((res, buf): Self::Output) -> Self {
        Self(Ok(res), buf)
    }

    fn branch(self) -> ControlFlow<Self::Residual, Self::Output> {
        match self {
            BufResult(Ok(res), buf) => ControlFlow::Continue((res, buf)),
            BufResult(Err(e), buf) => ControlFlow::Break(BufResult(Err(e), buf)),
        }
    }
}

#[cfg(feature = "try_trait_v2")]
impl<T, B> Residual<(T, B)> for BufResult<Infallible, B> {
    type TryType = BufResult<T, B>;
}

/// A helper macro to imitate the behavior of try trait `?`.
/// ```
/// # use compio_buf::{buf_try, BufResult};
/// fn foo() -> BufResult<i32, i32> {
///     let (a, b) = buf_try!(BufResult(Ok(1), 2));
///     assert_eq!(a, 1);
///     assert_eq!(b, 2);
///     (Ok(3), 4).into()
/// }
/// assert!(foo().is_ok());
/// ```
#[macro_export]
macro_rules! buf_try {
    ($e:expr) => {{
        match $e {
            $crate::BufResult(Ok(res), buf) => (res, buf),
            $crate::BufResult(Err(e), buf) => return $crate::BufResult(Err(e), buf),
        }
    }};
    ($e:expr, $b:expr) => {{
        let buf = $b;
        match $e {
            Ok(res) => (res, buf),
            Err(e) => return $crate::BufResult(Err(e), buf),
        }
    }};
}
