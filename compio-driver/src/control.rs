#![allow(dead_code)]

use std::{
    mem::{ManuallyDrop, MaybeUninit},
    ptr,
};

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
    control: MaybeUninit<ControlInner<T>>,
    op: T,
}

impl<T: OpCode> IntoInner for Carrier<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        let mut this = ManuallyDrop::new(self);
        unsafe {
            // SAFETY: `new_uninit` ensures that the type is initialized
            MaybeUninit::assume_init_drop(&mut this.control);
            // SAFETY: `self` is warpped in ManuallyDrop and is not used after here
            ptr::read(&this.op)
        }
    }
}

impl<T: OpCode> Carrier<T> {
    /// Create a new Carrier with given OpCode.
    ///
    /// # Safety
    ///
    /// Returned [`Carrier`] must be [`initialized`] before converting to `dyn
    /// Carry` or calling any of the `as_` getters.
    ///
    /// [`initialized`]: Self::init
    pub unsafe fn new_uninit(op: T) -> Self {
        Self {
            control: MaybeUninit::uninit(),
            op,
        }
    }

    /// Init the Carrier
    ///
    /// # Safety
    ///
    /// `self` must have stable address until control is no longer used, include
    /// all function calls via `dyn Carry` and `as_` getters.
    pub unsafe fn init(&mut self, driver_ty: DriverType) {
        #[cfg(fusion)]
        {
            let control = match driver_ty {
                DriverType::Poll => ControlInner::Poll(unsafe { PollOpCode::init(&mut self.op) }),
                DriverType::IoUring => {
                    ControlInner::IoUring(unsafe { IourOpCode::init(&mut self.op) })
                }
                DriverType::IOCP => unreachable!("Cannot be windows"),
            };

            self.control.write(control);
        }

        #[cfg(not(fusion))]
        {
            _ = driver_ty;

            let control = unsafe { OpCode::init(&mut self.op) };
            self.control.write(control);
        }
    }

    #[cfg(io_uring)]
    pub fn as_iour(&mut self) -> (&mut T, &mut <T as IourOpCode>::Control) {
        // SAFETY: `new_uninit` ensures that the type is initialized
        let control = unsafe { self.control.assume_init_mut() };
        #[cfg(fusion)]
        {
            (&mut self.op, control.iour())
        }
        #[cfg(not(fusion))]
        {
            (&mut self.op, control)
        }
    }

    #[cfg(polling)]
    pub fn as_poll(&mut self) -> (&mut T, &mut <T as PollOpCode>::Control) {
        // SAFETY: `new_uninit` ensures that the type is initialized
        let control = unsafe { self.control.assume_init_mut() };
        #[cfg(fusion)]
        {
            (&mut self.op, control.poll())
        }
        #[cfg(not(fusion))]
        {
            (&mut self.op, control)
        }
    }

    #[cfg(windows)]
    pub fn as_iocp(&self) -> (&T, &<T as OpCode>::Control) {
        // SAFETY: `new_uninit` ensures that the type is initialized
        let control = unsafe { self.control.assume_init_ref() };

        (&self.op, control)
    }

    #[cfg(windows)]
    pub fn as_iocp_mut(&mut self) -> (&mut T, &mut <T as OpCode>::Control) {
        // SAFETY: `new_uninit` ensures that the type is initialized
        let control = unsafe { self.control.assume_init_mut() };

        (&mut self.op, control)
    }
}

impl<T: OpCode + ?Sized> Drop for Carrier<T> {
    fn drop(&mut self) {
        // SAFETY: `new_uninit` ensures that the type is initialized
        unsafe { MaybeUninit::assume_init_drop(&mut self.control) };
    }
}
