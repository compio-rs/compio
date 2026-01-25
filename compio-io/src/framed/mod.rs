//! Framed I/O operations.
//!
//! This module provides functionality for encoding and decoding frames
//! for network protocols and other stream-based communication.

use std::marker::PhantomData;

use compio_buf::IoBufMut;
use futures_util::FutureExt;

use crate::{AsyncRead, framed::codec::Decoder, util::Splittable};

pub mod codec;
pub mod frame;

mod read;
mod write;

const CONFIG_POLLED_ERROR: &str = "`Framed` should not be configured after being polled";
const INCONSISTENT_ERROR: &str = "`Framed` is in an inconsistent state";

#[cold]
fn panic_config_polled() -> ! {
    panic!("{}", CONFIG_POLLED_ERROR);
}

/// A framed encoder/decoder that handles both [`Sink`] for writing frames and
/// [`Stream`] for reading frames.
///
/// It uses a [`codec`] to encode/decode messages into/from bytes (`T <-->
/// IoBufMut`) and a [`Framer`] to define how frames are laid out in buffer
/// (`&[u8] <--> IoBufMut`).
///
/// [`Framer`]: frame::Framer
/// [`Sink`]: futures_util::Sink
/// [`Stream`]: futures_util::Stream
pub struct Framed<R, W, C, F, In, Out, B = Vec<u8>> {
    read_state: read::State<R, B>,
    write_state: write::State<W, B>,
    codec: C,
    framer: F,
    types: PhantomData<(In, Out)>,
}

/// [`Framed`] with same `In` ([`Sink`]) and `Out` ([`Stream::Item`]) type
///
/// [`Sink`]: futures_util::Sink
/// [`Stream::Item`]: futures_util::Stream::Item
pub type SymmetricFramed<R, W, C, F, T, B = Vec<u8>> = Framed<R, W, C, F, T, T, B>;

impl<R, W, C, F, In, Out, B> Framed<R, W, C, F, In, Out, B> {
    /// Change the reader of the `Framed` object.
    pub fn with_reader<Io>(self, reader: Io) -> Framed<Io, W, C, F, In, Out, B> {
        Framed {
            read_state: self.read_state.with_io(reader),
            write_state: self.write_state,
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }

    /// Change the writer of the `Framed` object.
    pub fn with_writer<Io>(self, writer: Io) -> Framed<R, Io, C, F, In, Out, B> {
        Framed {
            read_state: self.read_state,
            write_state: self.write_state.with_io(writer),
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }

    /// Change the codec of the `Framed` object.
    ///
    /// This is useful when you have a duplex I/O type, e.g., a
    /// `compio::net::TcpStream` or `compio::fs::File`, and you want
    /// [`Framed`] to implement both [`Sink`](futures_util::Sink) and
    /// [`Stream`](futures_util::Stream).
    ///
    /// Some types like the ones mentioned above are multiplexed by nature, so
    /// they implement the [`Splittable`] trait by themselves. For other types,
    /// you may want to wrap them in [`Split`] first, which uses lock or
    /// `RefCell` under the hood.
    ///
    /// [`Split`]: crate::util::split::Split
    pub fn with_duplex<Io: Splittable>(
        self,
        io: Io,
    ) -> Framed<Io::ReadHalf, Io::WriteHalf, C, F, In, Out, B> {
        let (read_half, write_half) = io.split();

        Framed {
            read_state: self.read_state.with_io(read_half),
            write_state: self.write_state.with_io(write_half),
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }

    /// Change both the read and write buffers of the `Framed` object.
    ///
    /// This is useful when you want to provide custom buffers for reading and
    /// writing.
    pub fn with_buffer<Buf: IoBufMut>(
        self,
        read_buffer: Buf,
        write_buffer: Buf,
    ) -> Framed<R, W, C, F, In, Out, Buf> {
        Framed {
            read_state: self.read_state.with_buf(read_buffer),
            write_state: self.write_state.with_buf(write_buffer),
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }
}

impl<C, F> Framed<(), (), C, F, (), (), ()> {
    /// Creates a new `Framed` with the given I/O object, codec, framer and a
    /// different input and output type.
    pub fn new<In, Out>(codec: C, framer: F) -> Framed<(), (), C, F, In, Out> {
        Framed {
            read_state: read::State::empty(),
            write_state: write::State::empty(),
            codec,
            framer,
            types: PhantomData,
        }
    }

    /// Creates a new `Framed` with the given I/O object, codec, and framer with
    /// the same input and output type.
    pub fn symmetric<T>(codec: C, framer: F) -> Framed<(), (), C, F, T, T> {
        Framed {
            read_state: read::State::empty(),
            write_state: write::State::empty(),
            codec,
            framer,
            types: PhantomData,
        }
    }
}
