// TODO: Retrieve session

use std::{future::Future, io, pin::Pin};

type IoResultFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + 'a>>;

#[allow(async_fn_in_trait)]
pub trait SessionStorage: 'static {
    async fn store_session(&mut self, session: &[u8]) -> io::Result<()>;
}

pub(crate) trait DynSessionStorage: 'static {
    fn store_session_dyn<'a>(&'a mut self, session: &'a [u8]) -> IoResultFuture<'a, ()>;
}

impl<S: SessionStorage> DynSessionStorage for S {
    fn store_session_dyn<'a>(&'a mut self, session: &'a [u8]) -> IoResultFuture<'a, ()> {
        Box::pin(self.store_session(session))
    }
}

impl<S: SessionStorage> From<S> for Box<dyn DynSessionStorage> {
    fn from(storage: S) -> Self {
        Box::new(storage)
    }
}
