use crate::{
    OpCode,
    sys::{op::*, prelude::*},
};

impl<T: IoBufMut, S: AsFd> OpCode for ReadAt<T, S> {
    type Control = ();
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectoredAt<T, S> {
    type Control = ();
}

impl<T: IoBuf, S: AsFd> OpCode for WriteAt<T, S> {
    type Control = ();
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectoredAt<T, S> {
    type Control = ();
}

impl<T: IoBufMut, S: AsFd> OpCode for Read<T, S> {
    type Control = ();
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for ReadVectored<T, S> {
    type Control = ();
}

impl<T: IoBuf, S: AsFd> OpCode for Write<T, S> {
    type Control = ();
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for WriteVectored<T, S> {
    type Control = ();
}

impl<S: AsFd> OpCode for PollOnce<S> {
    type Control = ();
}

impl OpCode for Pipe {
    type Control = ();
}
