use crate::{
    OpCode,
    sys::{op::*, prelude::*},
};

impl<S: AsFd> OpCode for ReadManagedAt<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for ReadManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for RecvManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for RecvFromManaged<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<C: IoBufMut, S: AsFd> OpCode for RecvMsgManaged<C, S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for RecvFromMulti<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}

impl<S: AsFd> OpCode for RecvMsgMulti<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}
