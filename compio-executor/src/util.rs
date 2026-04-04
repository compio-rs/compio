use std::{process::abort, task::Poll, thread::panicking};

/// Calls a function and aborts if it panics.
///
/// This is useful in unsafe code where we can't recover from panics.
#[inline(always)]
pub(crate) fn abort_on_panic<T>(f: impl FnOnce() -> T) -> T {
    struct AbortOnPanic;

    impl Drop for AbortOnPanic {
        fn drop(&mut self) {
            if panicking() {
                abort()
            }
        }
    }

    let _b = AbortOnPanic;
    f()
}

#[inline(always)]
pub(crate) fn transpose<T, E>(poll: Result<Poll<T>, E>) -> Poll<Result<T, E>> {
    match poll {
        Ok(Poll::Pending) => Poll::Pending,
        Ok(Poll::Ready(t)) => Poll::Ready(Ok(t)),
        Err(e) => Poll::Ready(Err(e)),
    }
}

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

pub(crate) use assert_not_impl;
