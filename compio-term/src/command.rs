use std::{fmt, io, mem};

use compio_buf::{BufResult, IntoInner, IoBufExt};
use compio_io::AsyncWrite;

use crate::Command;

/// Adds terminal commands to an asynchronous command queue.
///
/// Queuing only renders the command into the queue's memory buffer. Call
/// [`CommandQueue::flush`] to write the complete batch to its asynchronous
/// writer.
pub trait QueueableCommand {
    /// Adds a command to the queue without writing to the underlying writer.
    ///
    /// If the command cannot render its ANSI representation, this method
    /// removes any bytes that the failed command added. Commands that were
    /// already queued remain intact.
    fn queue(&mut self, command: impl Command) -> io::Result<&mut Self>;
}

/// A buffered queue of terminal commands for a Compio asynchronous writer.
///
/// Commands are encoded with [`Command::write_ansi`] and written in one ordered
/// batch when [`flush`](Self::flush) is awaited. The writer can be Compio's
/// standard output handle or any other type that implements
/// [`compio_io::AsyncWrite`].
///
/// Call [`flush`](Self::flush) before this value is dropped. Dropping it
/// discards commands that are still queued.
///
/// # Example
///
/// ```no_run
/// use std::io;
///
/// use compio_term::{CommandQueue, QueueableCommand, cursor::MoveTo, style::Print};
///
/// #[compio_macros::main]
/// async fn main() -> io::Result<()> {
///     let mut output = CommandQueue::stdout();
///     output.queue(MoveTo(0, 0))?.queue(Print("ready\r\n"))?;
///     output.flush().await
/// }
/// ```
#[derive(Debug)]
#[must_use = "queued commands are discarded unless flush is awaited"]
pub struct CommandQueue<W> {
    writer: W,
    buffer: Vec<u8>,
    written: usize,
}

impl<W> CommandQueue<W> {
    /// Creates an empty command queue for `writer`.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            buffer: Vec::new(),
            written: 0,
        }
    }

    /// Creates an empty command queue with space for at least `capacity` bytes.
    pub fn with_capacity(capacity: usize, writer: W) -> Self {
        Self {
            writer,
            buffer: Vec::with_capacity(capacity),
            written: 0,
        }
    }

    /// Returns the number of command bytes that have not been written.
    pub fn buffered_len(&self) -> usize {
        self.buffer.len() - self.written
    }

    /// Returns `true` when no command bytes are waiting to be written.
    pub fn is_empty(&self) -> bool {
        self.buffered_len() == 0
    }
}

impl CommandQueue<compio_fs::Stdout> {
    /// Creates an empty command queue that writes to standard output.
    pub fn stdout() -> Self {
        Self::new(compio_fs::stdout())
    }
}

impl<W: AsyncWrite> CommandQueue<W> {
    /// Writes all queued commands and flushes the underlying writer.
    ///
    /// A partial write is resumed at the first unwritten byte if this method is
    /// called again after a write error. Commands queued after that error stay
    /// after the remaining bytes from the failed batch.
    pub async fn flush(&mut self) -> io::Result<()> {
        while self.written < self.buffer.len() {
            let buffer = mem::take(&mut self.buffer).slice(self.written..);
            let BufResult(result, buffer) = self.writer.write(buffer).await;
            self.buffer = buffer.into_inner();

            match result {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to write queued terminal commands",
                    ));
                }
                Ok(written) => {
                    let remaining = self.buffer.len() - self.written;
                    if written > remaining {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "asynchronous writer reported too many written bytes",
                        ));
                    }
                    self.written += written;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }

        self.buffer.clear();
        self.written = 0;
        self.writer.flush().await
    }
}

impl<W> QueueableCommand for CommandQueue<W> {
    fn queue(&mut self, command: impl Command) -> io::Result<&mut Self> {
        let original_len = self.buffer.len();
        if command
            .write_ansi(&mut AnsiBuffer(&mut self.buffer))
            .is_err()
        {
            self.buffer.truncate(original_len);
            return Err(io::Error::other(
                "terminal command failed to render its ANSI representation",
            ));
        }
        Ok(self)
    }
}

struct AnsiBuffer<'a>(&'a mut Vec<u8>);

impl fmt::Write for AnsiBuffer<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.0.extend_from_slice(value.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fmt, io, rc::Rc};

    use compio_buf::{BufResult, IoBuf, IoBufExt};
    use compio_io::AsyncWrite;
    use futures_util::FutureExt;

    use super::{CommandQueue, QueueableCommand};
    use crate::{Command, cursor::MoveTo, style::Print};

    #[test]
    fn queues_commands_until_flush_in_order() {
        let writer = TestWriter::new(2, None);
        let state = writer.state();
        let mut queue = CommandQueue::new(writer);
        let expected = b"\x1b[5;4Hready";

        queue
            .queue(MoveTo(3, 4))
            .unwrap()
            .queue(Print("ready"))
            .unwrap();

        assert_eq!(queue.buffered_len(), expected.len());
        assert!(state.borrow().output.is_empty());
        flush(&mut queue).unwrap();
        assert_eq!(&state.borrow().output, expected);
        assert!(queue.is_empty());
    }

    #[test]
    fn failed_command_does_not_leave_partial_bytes() {
        let writer = TestWriter::new(usize::MAX, None);
        let state = writer.state();
        let mut queue = CommandQueue::new(writer);

        queue.queue(Print("before")).unwrap();
        let error = queue
            .queue(FailingCommand)
            .err()
            .expect("failing command must return an error");
        queue.queue(Print("after")).unwrap();
        flush(&mut queue).unwrap();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(&state.borrow().output, b"beforeafter");
    }

    #[test]
    fn retry_resumes_after_a_partial_write() {
        let writer = TestWriter::new(2, Some(2));
        let state = writer.state();
        let mut queue = CommandQueue::new(writer);
        queue.queue(Print("abcdef")).unwrap();

        let error = flush(&mut queue).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(&state.borrow().output, b"ab");

        queue.queue(Print("gh")).unwrap();
        flush(&mut queue).unwrap();
        assert_eq!(&state.borrow().output, b"abcdefgh");
    }

    fn flush(queue: &mut CommandQueue<TestWriter>) -> io::Result<()> {
        queue
            .flush()
            .now_or_never()
            .expect("test writer must not yield")
    }

    struct FailingCommand;

    impl Command for FailingCommand {
        fn write_ansi(&self, writer: &mut impl fmt::Write) -> fmt::Result {
            fmt::Write::write_str(writer, "partial")?;
            Err(fmt::Error)
        }

        #[cfg(windows)]
        fn execute_winapi(&self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct TestWriter {
        state: Rc<RefCell<WriterState>>,
    }

    impl TestWriter {
        fn new(chunk_size: usize, fail_at: Option<usize>) -> Self {
            Self {
                state: Rc::new(RefCell::new(WriterState {
                    output: Vec::new(),
                    chunk_size,
                    writes: 0,
                    fail_at,
                })),
            }
        }

        fn state(&self) -> Rc<RefCell<WriterState>> {
            Rc::clone(&self.state)
        }
    }

    struct WriterState {
        output: Vec<u8>,
        chunk_size: usize,
        writes: usize,
        fail_at: Option<usize>,
    }

    impl AsyncWrite for TestWriter {
        async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
            let mut state = self.state.borrow_mut();
            state.writes += 1;
            if state.fail_at == Some(state.writes) {
                state.fail_at = None;
                return BufResult(Err(io::Error::other("injected write failure")), buffer);
            }

            let written = state.chunk_size.min(buffer.buf_len());
            state.output.extend_from_slice(&buffer.as_init()[..written]);
            BufResult(Ok(written), buffer)
        }

        async fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }

        async fn shutdown(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
