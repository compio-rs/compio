use std::fmt;

use crate::Command;

/// An ordered batch of terminal commands.
///
/// Implemented for borrowed collections that yield shared [`Command`]
/// references and for heterogeneous tuples of up to 20 commands.
pub trait Commands {
    /// Writes the ANSI representation of every command in order.
    ///
    /// This method is normally called through [`CommandQueue::queue_many`].
    ///
    /// [`CommandQueue::queue_many`]: crate::CommandQueue::queue_many
    fn write_ansi(&self, writer: &mut impl fmt::Write) -> fmt::Result;
}

impl<'a, C, I: ?Sized> Commands for &'a I
where
    C: Command + 'a,
    &'a I: IntoIterator<Item = &'a C>,
{
    fn write_ansi(&self, writer: &mut impl fmt::Write) -> fmt::Result {
        for command in *self {
            command.write_ansi(writer)?;
        }
        Ok(())
    }
}

macro_rules! impl_commands_for_tuples {
    (($head_type:ident, $head:ident) $(, ($tail_type:ident, $tail:ident))*) => {
        impl<$head_type: Command $(, $tail_type: Command)*> Commands
            for ($head_type, $($tail_type,)*)
        {
            fn write_ansi(&self, writer: &mut impl fmt::Write) -> fmt::Result {
                let ($head, $($tail,)*) = self;
                $head.write_ansi(writer)?;
                $($tail.write_ansi(writer)?;)*
                Ok(())
            }
        }

        impl_commands_for_tuples!($(($tail_type, $tail)),*);
    };
    () => {
        impl Commands for () {
            fn write_ansi(&self, _: &mut impl fmt::Write) -> fmt::Result {
                Ok(())
            }
        }
    };
}

impl_commands_for_tuples!(
    (C0, c0),
    (C1, c1),
    (C2, c2),
    (C3, c3),
    (C4, c4),
    (C5, c5),
    (C6, c6),
    (C7, c7),
    (C8, c8),
    (C9, c9),
    (C10, c10),
    (C11, c11),
    (C12, c12),
    (C13, c13),
    (C14, c14),
    (C15, c15),
    (C16, c16),
    (C17, c17),
    (C18, c18),
    (C19, c19)
);
