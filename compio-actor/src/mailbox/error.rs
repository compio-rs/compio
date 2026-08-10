use std::{error::Error, fmt};

use super::Call;
use crate::Message;

/// A message rejected by a mailbox or broker.
#[derive(Debug, PartialEq, Eq)]
pub enum DeliverError<M: Message> {
    /// The mailbox is at capacity.
    Full(M),
    /// The actor is stopping or has exited.
    Closed(M),
}

impl<M: Message> DeliverError<M> {
    /// Recovers the rejected message.
    pub fn into_inner(self) -> M {
        match self {
            Self::Full(message) | Self::Closed(message) => message,
        }
    }
}

impl<M: Message> fmt::Display for DeliverError<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.write_str("actor mailbox is full"),
            Self::Closed(_) => f.write_str("actor mailbox is closed"),
        }
    }
}

impl<M: Message + fmt::Debug> Error for DeliverError<M> {}

/// A call that could not be delivered or answered.
#[derive(Debug, PartialEq, Eq)]
pub enum CallError<M: Message> {
    /// The mailbox was at capacity.
    Full(M),
    /// The actor was stopping or had exited.
    Closed(M),
    /// The actor handled the request without replying.
    NoReply,
}

impl<M: Message> CallError<M> {
    pub(super) fn from_deliver<R: Message>(error: DeliverError<Call<M, R>>) -> Self {
        match error {
            DeliverError::Full(call) => Self::Full(call.into_message()),
            DeliverError::Closed(call) => Self::Closed(call.into_message()),
        }
    }

    /// Recovers a request that was not delivered.
    pub fn into_inner(self) -> Option<M> {
        match self {
            Self::Full(message) | Self::Closed(message) => Some(message),
            Self::NoReply => None,
        }
    }
}

impl<M: Message> fmt::Display for CallError<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.write_str("actor mailbox is full"),
            Self::Closed(_) => f.write_str("actor mailbox is closed"),
            Self::NoReply => f.write_str("actor did not reply"),
        }
    }
}

impl<M: Message + fmt::Debug> Error for CallError<M> {}
