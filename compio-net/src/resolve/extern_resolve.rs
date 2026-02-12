use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll, Waker},
};

unsafe extern "Rust" {
    /// Create resolver state, returns an opaque handle.
    fn __compio_resolve_create(host: &str, port: u16) -> *mut ();

    /// Poll for resolution result. If `Pending`, the implementor must call `waker.wake()` when ready.
    fn __compio_resolve_poll(
        handle: *mut (),
        waker: &Waker,
    ) -> Poll<io::Result<Vec<SocketAddr>>>;

    /// Drop resolver state.
    fn __compio_resolve_drop(handle: *mut ());
}

/// Zero-overhead future wrapping extern resolve functions.
struct ExternResolveFuture {
    handle: *mut (),
}

impl Future for ExternResolveFuture {
    type Output = io::Result<Vec<SocketAddr>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { __compio_resolve_poll(self.handle, cx.waker()) }
    }
}

impl Drop for ExternResolveFuture {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { __compio_resolve_drop(self.handle) }
        }
    }
}

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {

    let handle = unsafe { __compio_resolve_create(host, port) };
    let future = ExternResolveFuture { handle };
    future.await.map(|addrs| addrs.into_iter())
}

/// Trait that extern resolvers must implement.
///
/// The [`resolve_set!`] macro checks this at compile time,
/// so missing methods produce clear errors instead of linker failures.
pub trait ExternResolve {
    /// Create a new resolver for the given host and port.
    fn new(host: &str, port: u16) -> Self;

    /// Poll for the resolution result.
    ///
    /// If not yet ready, register the `waker` and return `Poll::Pending`.
    /// The implementation must call `waker.wake()` when results become available.
    fn poll(&mut self, waker: &Waker) -> Poll<io::Result<Vec<SocketAddr>>>;
}

/// Register a custom async DNS resolver implementation.
///
/// The provided type must implement [`ExternResolve`].
///
/// # Usage
///
/// ```ignore
/// struct MyResolver { /* ... */ }
///
/// impl compio_net::ExternResolve for MyResolver {
///     fn new(host: &str, port: u16) -> Self {
///         // Start async DNS query
///         todo!()
///     }
///
///     fn poll(
///         &mut self,
///         waker: &std::task::Waker,
///     ) -> std::task::Poll<std::io::Result<Vec<std::net::SocketAddr>>> {
///         // Check if ready, otherwise register waker
///         todo!()
///     }
/// }
///
/// compio_net::resolve_set!(MyResolver);
/// ```
#[macro_export]
macro_rules! resolve_set {
    ($resolver:ty) => {
        // Compile-time constraint check: ensure $resolver implements ExternResolve.
        // If not, the compiler will emit a clear error here instead of a cryptic linker error.
        const _: () = {
            fn _assert_impl<T: $crate::ExternResolve>() {}
            fn _check() { _assert_impl::<$resolver>(); }
        };

        #[unsafe(no_mangle)]
        pub fn __compio_resolve_create(host: &str, port: u16) -> *mut () {
            let resolver = ::std::boxed::Box::new(<$resolver as $crate::ExternResolve>::new(host, port));
            ::std::boxed::Box::into_raw(resolver) as *mut ()
        }

        #[unsafe(no_mangle)]
        pub fn __compio_resolve_poll(
            handle: *mut (),
            waker: &::std::task::Waker,
        ) -> ::std::task::Poll<
            ::std::io::Result<::std::vec::Vec<::std::net::SocketAddr>>,
        > {
            let resolver = unsafe { &mut *(handle as *mut $resolver) };
            $crate::ExternResolve::poll(resolver, waker)
        }

        #[unsafe(no_mangle)]
        pub fn __compio_resolve_drop(handle: *mut ()) {
            let _ = unsafe { ::std::boxed::Box::from_raw(handle as *mut $resolver) };
        }
    };
}

