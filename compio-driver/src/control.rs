use std::mem::MaybeUninit;

use thin_cell::unsync::ThinCell;

use crate::{DriverType, OpCode};

cfg_if::cfg_if! {
    if #[cfg(fusion)] {
        use crate::{PollOpCode, IourOpCode};
    } else if #[cfg(feature = "polling")] {
        use crate::OpCode as PollOpCode;
    } else {
        use crate::OpCode as IoUrOpCode;
    }
}

#[cfg(not(fusion))]
type ControlInner<T: OpCode> = T::Control;

#[cfg(fusion)]
enum ControlInner<T: OpCode> {
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
/// The only way to access this type is through [`ThinCell`], which pins it on
/// the heap and guarantees any self-referential pointers to be valid.
pub(crate) struct Material<T: OpCode> {
    op: T,
    control: MaybeUninit<ControlInner<T>>,
}

impl<T: OpCode> Material<T> {
    pub fn new(op: T, driver_ty: DriverType) -> ThinCell<Material<T>> {
        let cell = ThinCell::new(Material {
            op,
            control: MaybeUninit::uninit(),
        });

        #[cfg(fusion)]
        {
            let mut borrowed = cell.borrow();
            let control = match driver_ty {
                DriverType::Poll => {
                    ControlInner::Poll(unsafe { PollOpCode::init(&mut borrowed.op) })
                }
                DriverType::IoUring => {
                    ControlInner::IoUring(unsafe { IourOpCode::init(&mut borrowed.op) })
                }
                DriverType::IOCP => unreachable!("Cannot be windows"),
            };

            borrowed.control.write(control);
        }

        #[cfg(not(fusion))]
        {
            _ = driver_ty;
            let mut borrowed = cell.borrow();
            let control = unsafe { OpCode::init(&mut borrowed.op) };
            borrowed.control.write(control);
        }

        cell
    }

    #[cfg(io_uring)]
    pub fn as_iour(&mut self) -> (&mut T, &mut <T as IourOpCode>::Control) {
        // SAFETY: control is initialized in `new`
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

    #[cfg(feature = "polling")]
    pub fn as_poll(&mut self) -> (&mut T, &mut <T as PollOpCode>::Control) {
        // SAFETY: control is initialized in `new`
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
}
