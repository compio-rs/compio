use windows_sys::Win32::System::IO::OVERLAPPED;

use crate::{OpCode, OpType, sys::op::*};

unsafe impl<D, F> OpCode for Asyncify<F, D>
where
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd);
        self.data = Some(data);
        Poll::Ready(res)
    }
}

unsafe impl<S1, S2, D, F> OpCode for AsyncifyFd2<S1, S2, F, D>
where
    S1: std::marker::Sync,
    S2: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S1, &S2) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // SAFETY: self won't be moved
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd1, &self.fd2);
        self.data = Some(data);
        Poll::Ready(res)
    }
}
