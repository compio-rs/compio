use std::{
    any::Any,
    error::Error,
    fmt::{Debug, Display},
    io,
    panic::{UnwindSafe, catch_unwind, resume_unwind},
};

pub(crate) struct Panic(Box<dyn Any + Send>);

// Panic is unconditionally `Sync`
unsafe impl Sync for Panic {}

impl Display for Panic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self, f)
    }
}

impl Debug for Panic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Panic").finish_non_exhaustive()
    }
}

impl Error for Panic {}

impl From<Panic> for io::Error {
    fn from(value: Panic) -> Self {
        Self::other(value)
    }
}

pub(crate) fn catch_unwind_io<F, R>(f: F) -> io::Result<R>
where
    F: FnOnce() -> io::Result<R> + UnwindSafe,
{
    catch_unwind(f).map_err(|err| io::Error::from(Panic(err)))?
}

pub(crate) fn resume_unwind_io<T>(res: io::Result<T>) -> io::Result<T> {
    let Err(e) = res else { return res };
    match e.downcast::<Panic>() {
        Ok(p) => resume_unwind(p.0),
        Err(e) => Err(e),
    }
}
