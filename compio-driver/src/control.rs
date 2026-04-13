#![allow(dead_code)]

use compio_buf::IntoInner;

use crate::{DriverType, OpCode};

cfg_if::cfg_if! {
    if #[cfg(fusion)] {
        use crate::{PollOpCode, IourOpCode};
    } else if #[cfg(io_uring)] {
        use crate::OpCode as IourOpCode;
    } else if #[cfg(polling)]{
        use crate::OpCode as PollOpCode;
    }
}

#[cfg(not(fusion))]
type ControlInner<T> = <T as OpCode>::Control;

#[cfg(fusion)]
enum ControlInner<T: OpCode + ?Sized> {
    Poll(<T as PollOpCode>::Control),
    IoUring(<T as IourOpCode>::Control),
}

#[cfg(fusion)]
impl<T: OpCode> ControlInner<T> {
    pub fn iour(&mut self) -> &mut <T as IourOpCode>::Control {
        match self {
            ControlInner::Poll(_) => unreachable!("Current driver is not `io-uring`"),
            ControlInner::IoUring(control) => control,
        }
    }

    pub fn poll(&mut self) -> &mut <T as PollOpCode>::Control {
        match self {
            ControlInner::Poll(control) => control,
            ControlInner::IoUring(_) => unreachable!("Current driver is not `polling`"),
        }
    }
}

/// A utility type that put a [`OpCode`] and its [`OpCode::Control`] together.
///
/// The only way to access this type is through [`ErasedKey`], which pins it on
/// the heap and guarantees any self-referential pointers to be valid.
///
/// [`ErasedKey`]: crate::key::ErasedKey
pub(crate) struct Carrier<T: OpCode + ?Sized> {
    control: ControlInner<T>,
    driver_ty: DriverType,
    op: T,
}

impl<T: OpCode> IntoInner for Carrier<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.op
    }
}

impl<T: OpCode> Carrier<T> {
    /// Create a new Carrier with given OpCode and driver type.
    pub fn new(op: T, driver_ty: DriverType) -> Self {
        #[cfg(fusion)]
        let control = match driver_ty {
            DriverType::IoUring => ControlInner::IoUring(Default::default()),
            DriverType::Poll => ControlInner::Poll(Default::default()),
            _ => unreachable!("Cannot be IOCP"),
        };
        #[cfg(not(fusion))]
        let control = T::Control::default();
        Self {
            control,
            driver_ty,
            op,
        }
    }

    /// Init the Carrier
    ///
    /// # Safety
    ///
    /// `self` must have stable address until control is no longer used, include
    /// all function calls via `dyn Carry` and `as_` getters.
    pub unsafe fn init(&mut self) {
        #[cfg(fusion)]
        unsafe {
            match self.driver_ty {
                DriverType::Poll => PollOpCode::init(&mut self.op, self.control.poll()),
                DriverType::IoUring => IourOpCode::init(&mut self.op, self.control.iour()),
                _ => unreachable!("Cannot be IOCP"),
            };
        }

        #[cfg(not(fusion))]
        unsafe {
            OpCode::init(&mut self.op, &mut self.control)
        }
    }

    #[cfg(io_uring)]
    pub fn as_iour(&mut self) -> (&mut T, &mut <T as IourOpCode>::Control) {
        #[cfg(fusion)]
        return (&mut self.op, self.control.iour());
        #[cfg(not(fusion))]
        (&mut self.op, &mut self.control)
    }

    #[cfg(polling)]
    pub fn as_poll(&mut self) -> (&mut T, &mut <T as PollOpCode>::Control) {
        #[cfg(fusion)]
        return (&mut self.op, self.control.poll());
        #[cfg(not(fusion))]
        (&mut self.op, &mut self.control)
    }

    #[cfg(windows)]
    pub fn as_iocp(&self) -> (&T, &<T as OpCode>::Control) {
        (&self.op, &self.control)
    }

    #[cfg(windows)]
    pub fn as_iocp_mut(&mut self) -> (&mut T, &mut <T as OpCode>::Control) {
        (&mut self.op, &mut self.control)
    }
}
