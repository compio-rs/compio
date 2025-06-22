//! Framed I/O operations.
//!
//! This module provides functionality for encoding and decoding frames
//! for network protocols and other stream-based communication.

use std::marker::PhantomData;

use futures_util::FutureExt;

use crate::{AsyncRead, buffer::Buffer, framed::codec::Decoder, util::Splittable};

pub mod codec;
pub mod frame;

mod read;
mod write;

/// A framed encoder/decoder that handles both [`Sink`] for writing frames and
/// [`Stream`] for reading frames.
///
/// It uses a [`codec`] to encode/decode messages into frames (`T -> Vec<u8>`)
/// and a [`Framer`] to define how frames are laid out in buffer (`&mut [u8] ->
/// &mut [u8]`).
pub struct Framed<R, W, C, F, In, Out> {
    read_state: read::State<R>,
    write_state: write::State<W>,
    codec: C,
    framer: F,
    types: PhantomData<(In, Out)>,
}

/// [`Framed`] with same In ([`Sink`]) and Out ([`Stream::Item`]) type
pub type SymmetricFramed<R, W, C, F, Item> = Framed<R, W, C, F, Item, Item>;

impl<R, W, C, F, In, Out> Framed<R, W, C, F, In, Out> {
    /// Change the reader of the `Framed` object.
    pub fn with_reader<Io>(self, reader: Io) -> Framed<Io, W, C, F, In, Out> {
        Framed {
            read_state: read::State::Idle(Some((reader, Buffer::with_capacity(64)))),
            write_state: self.write_state,
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }

    /// Change the writer of the `Framed` object.
    pub fn with_writer<Io>(self, writer: Io) -> Framed<R, Io, C, F, In, Out> {
        Framed {
            read_state: self.read_state,
            write_state: write::State::Idle(Some((writer, Vec::new()))),
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }

    /// Change the codec of the `Framed` object.
    ///
    /// This is useful when you have a duplex I/O type, e.g., a `TcpStream` or
    /// `File`, and you want [`Framed`] to implement both
    /// [`Sink`](futures_util::Sink) and [`Stream`](futures_util::Stream).
    pub fn with_duplex<Io: Splittable>(
        self,
        io: Io,
    ) -> Framed<Io::ReadHalf, Io::WriteHalf, C, F, In, Out> {
        let (read_half, write_half) = io.split();

        Framed {
            read_state: read::State::Idle(Some((read_half, Buffer::with_capacity(64)))),
            write_state: write::State::Idle(Some((write_half, Vec::new()))),
            codec: self.codec,
            framer: self.framer,
            types: PhantomData,
        }
    }
}

impl<C, F> Framed<(), (), C, F, (), ()> {
    /// Creates a new `Framed` with the given I/O object, codec, framer and a
    /// different input and output type.
    pub fn new<In, Out>(codec: C, framer: F) -> Framed<(), (), C, F, In, Out> {
        Framed {
            read_state: read::State::Idle(None),
            write_state: write::State::Idle(None),
            codec,
            framer,
            types: PhantomData,
        }
    }

    /// Creates a new `Framed` with the given I/O object, codec, and framer with
    /// the same input and output type.
    pub fn symmetric<T>(codec: C, framer: F) -> Framed<(), (), C, F, T, T> {
        Framed {
            read_state: read::State::Idle(None),
            write_state: write::State::Idle(None),
            codec,
            framer,
            types: PhantomData,
        }
    }
}
