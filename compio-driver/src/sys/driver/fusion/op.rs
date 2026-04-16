use super::*;

/// Fused [`OpCode`]
///
/// This trait encapsulates both operation for `io-uring` and `polling`
pub trait OpCode: PollOpCode + IourOpCode {}

impl<T: PollOpCode + IourOpCode + ?Sized> OpCode for T {}

pub(crate) trait Carry: iour::Carry + poll::Carry {
    unsafe fn set_result(&mut self, result: &io::Result<usize>, extra: &crate::Extra);
}

impl<C: iour::Carry + poll::Carry> Carry for C {
    unsafe fn set_result(&mut self, result: &io::Result<usize>, extra: &crate::Extra) {
        unsafe {
            if extra.is_iour() {
                iour::Carry::set_result(self, result, extra);
            } else {
                poll::Carry::set_result(self, result, extra);
            }
        }
    }
}
