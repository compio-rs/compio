use crate::{
    OpCode,
    sys::{op::*, prelude::*},
};

impl OpCode for CreateSocket {
    type Control = ();
}

impl<S: AsFd> OpCode for Bind<S> {
    type Control = ();
}

impl<S: AsFd> OpCode for Listen<S> {
    type Control = ();
}

impl<S: AsFd> OpCode for ShutdownSocket<S> {
    type Control = ();
}

impl OpCode for CloseSocket {
    type Control = ();
}

impl<S: AsFd> OpCode for Accept<S> {
    type Control = ();
}

impl<S: AsFd> OpCode for Connect<S> {
    type Control = ();
}

impl<T: IoBufMut, S: AsFd> OpCode for Recv<T, S> {
    type Control = ();
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvVectored<T, S> {
    type Control = ();
}

impl<T: IoBuf, S: AsFd> OpCode for Send<T, S> {
    type Control = ();
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendVectored<T, S> {
    type Control = ();
}

impl<T: IoBufMut, S: AsFd> OpCode for RecvFrom<T, S> {
    type Control = ();
}

impl<T: IoVectoredBufMut, S: AsFd> OpCode for RecvFromVectored<T, S> {
    type Control = ();
}

impl<T: IoBuf, S: AsFd> OpCode for SendTo<T, S> {
    type Control = ();
}

impl<T: IoVectoredBuf, S: AsFd> OpCode for SendToVectored<T, S> {
    type Control = ();
}

impl<T: IoVectoredBufMut, C: IoBufMut, S: AsFd> OpCode for RecvMsg<T, C, S> {
    type Control = ();
}

impl<T: IoVectoredBuf, C: IoBuf, S: AsFd> OpCode for SendMsg<T, C, S> {
    type Control = ();
}
