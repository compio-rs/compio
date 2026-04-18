use crate::{
    OpCode,
    sys::{op::*, prelude::*},
};

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
> OpCode for Asyncify<F, D>
{
    type Control = ();
}

impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();
}

impl<S1, S2, D, F> OpCode for AsyncifyFd2<S1, S2, F, D>
where
    S1: std::marker::Sync,
    S2: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();
}
