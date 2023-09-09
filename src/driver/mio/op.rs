use std::io;

use mio::event::Event;

pub use crate::driver::unix_op::*;
use crate::{
    buf::{AsIoSlices, AsIoSlicesMut, IoBuf, IoBufMut},
    driver::{syscall, Decision, OpCode},
    op::*,
};

impl<T: IoBufMut> OpCode for ReadAt<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl OpCode for Sync {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::Completed(0))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        unreachable!("Sync operation should not be submitted to mio")
    }
}

impl OpCode for Accept {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl OpCode for Connect {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        let res = syscall!(connect(self.fd, self.addr.as_ptr(), self.addr.len(),))?;
        Ok(Decision::Completed(res))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_readable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl<T: AsIoSlices> OpCode for SendImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::wait_writable(self.fd))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl<T: AsIoSlicesMut> OpCode for RecvFromImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::Completed(0))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}

impl<T: AsIoSlices> OpCode for SendToImpl<T> {
    fn pre_submit(&mut self) -> io::Result<Decision> {
        Ok(Decision::Completed(0))
    }

    fn on_event(&mut self, event: &Event) -> std::io::Result<usize> {
        todo!()
    }
}
