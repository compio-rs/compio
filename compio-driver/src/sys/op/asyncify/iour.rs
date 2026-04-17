use super::*;
use crate::{IourOpCode as OpCode, OpEntry};

unsafe impl<D, F> OpCode for Asyncify<F, D>
where
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        self.data = Some(data);
        res
    }
}

unsafe impl<S, D, F> OpCode for AsyncifyFd<S, F, D>
where
    S: std::marker::Sync,
    D: std::marker::Send + 'static,
    F: (FnOnce(&S) -> BufResult<usize, D>) + std::marker::Send + 'static,
{
    type Control = ();

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd);
        self.data = Some(data);
        res
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

    fn create_entry(&mut self, _control: &mut Self::Control) -> OpEntry {
        OpEntry::Blocking
    }

    fn call_blocking(&mut self, _control: &mut Self::Control) -> std::io::Result<usize> {
        let f = self
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f(&self.fd1, &self.fd2);
        self.data = Some(data);
        res
    }
}
