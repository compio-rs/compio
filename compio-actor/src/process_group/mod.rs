//! Typed actor groups with configurable routing.

mod strategy;

use std::{
    fmt,
    sync::{Arc, Mutex, Weak},
};

#[doc(inline)]
pub use strategy::Strategy;

use crate::{
    Broker, Call, Message,
    mailbox::{CallError, DeliverError, call_with},
};

/// A group of actors that share messages using a routing [`Strategy`].
pub struct ProcessGroup<M: Message> {
    inner: Arc<GroupInner<M>>,
}

impl<M: Message> ProcessGroup<M> {
    /// Creates an empty process group.
    pub fn new() -> Self {
        Self::with_strategy(Strategy::default())
    }

    /// Creates an empty process group with a routing strategy.
    pub fn with_strategy(strategy: Strategy) -> Self {
        Self {
            inner: Arc::new(GroupInner {
                state: Mutex::new(GroupState {
                    next_id: 0,
                    cursor: 0,
                    members: Vec::new(),
                    strategy,
                }),
            }),
        }
    }

    /// Adds a broker until the returned membership is dropped.
    pub fn join(&self, broker: Broker<M>) -> Membership<M> {
        let mut state = self.inner.state.lock().unwrap();
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.members.push(Member { id, broker });
        Membership {
            id,
            group: Arc::downgrade(&self.inner),
        }
    }

    /// Routes a message to the next available member.
    pub fn send(&self, mut message: M) -> Result<(), DeliverError<M>> {
        let mut state = self.inner.state.lock().unwrap();
        let attempts = state.members.len();
        let mut attempted = 0;
        let mut saw_full = false;
        let mut index = match std::num::NonZeroUsize::new(state.members.len()) {
            Some(members) => {
                let strategy = state.strategy;
                strategy.select(&mut state.cursor, members)
            }
            None => return Err(DeliverError::Closed(message)),
        };

        while attempted < attempts && !state.members.is_empty() {
            attempted += 1;

            match state.members[index].broker.send(message) {
                Ok(()) => return Ok(()),
                Err(DeliverError::Full(returned)) => {
                    saw_full = true;
                    message = returned;
                    index = (index + 1) % state.members.len();
                }
                Err(DeliverError::Closed(returned)) => {
                    message = returned;
                    state.members.remove(index);
                    if !state.members.is_empty() {
                        index %= state.members.len();
                    }
                }
            }
        }

        if saw_full {
            Err(DeliverError::Full(message))
        } else {
            Err(DeliverError::Closed(message))
        }
    }

    /// Returns the number of registered members.
    pub fn len(&self) -> usize {
        self.inner.state.lock().unwrap().members.len()
    }

    /// Returns whether the group has no members.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<M: Message, R: Message> ProcessGroup<Call<M, R>> {
    /// Routes a request and waits for the selected actor's reply.
    pub async fn call(&self, message: M) -> Result<R, CallError<M>> {
        call_with(message, |call| self.send(call)).await
    }
}

impl<M: Message> Clone for ProcessGroup<M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<M: Message> Default for ProcessGroup<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Message> fmt::Debug for ProcessGroup<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessGroup")
            .field("members", &self.len())
            .finish()
    }
}

/// An actor's membership in a [`ProcessGroup`].
#[must_use = "dropping the membership removes the actor from the process group"]
pub struct Membership<M: Message> {
    id: u64,
    group: Weak<GroupInner<M>>,
}

impl<M: Message> Membership<M> {
    /// Removes the actor from the group.
    pub fn leave(self) {}
}

impl<M: Message> Drop for Membership<M> {
    fn drop(&mut self) {
        let Some(group) = self.group.upgrade() else {
            return;
        };
        let mut state = group.state.lock().unwrap();
        if let Some(index) = state.members.iter().position(|member| member.id == self.id) {
            state.members.remove(index);
        }
    }
}

impl<M: Message> fmt::Debug for Membership<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Membership").field("id", &self.id).finish()
    }
}

struct GroupInner<M: Message> {
    state: Mutex<GroupState<M>>,
}

struct GroupState<M: Message> {
    next_id: u64,
    cursor: usize,
    members: Vec<Member<M>>,
    strategy: Strategy,
}

struct Member<M: Message> {
    id: u64,
    broker: Broker<M>,
}
