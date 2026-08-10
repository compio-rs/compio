use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use crate::{
    Actor, Mailbox,
    mailbox::{ErasedMailbox, Name},
};

#[derive(Default)]
pub(super) struct Registry {
    state: OnceLock<Arc<RegistryState>>,
}

#[derive(Default)]
struct RegistryState {
    actors: Mutex<HashMap<Name, Option<ErasedMailbox>>>,
}

impl Registry {
    pub(super) fn reserve(&self, name: Name) -> Result<Registration, Name> {
        let state = self
            .state
            .get_or_init(|| Arc::new(RegistryState::default()));
        let mut actors = state.actors.lock().unwrap();
        if actors.contains_key(name.as_str()) {
            return Err(name);
        }
        actors.insert(name.clone(), None);
        Ok(Registration {
            name,
            state: state.clone(),
        })
    }

    pub(super) fn get<A>(&self, name: &str) -> Option<Mailbox<A>>
    where
        A: Actor,
    {
        let mailbox = self
            .state
            .get()?
            .actors
            .lock()
            .unwrap()
            .get(name)
            .and_then(Clone::clone)?;
        Mailbox::from_erased(mailbox)
    }
}

pub(super) struct Registration {
    name: Name,
    state: Arc<RegistryState>,
}

impl Registration {
    pub(super) fn activate<A>(&self, mailbox: &Mailbox<A>)
    where
        A: Actor,
    {
        let mut actors = self.state.actors.lock().unwrap();
        let actor = actors
            .get_mut(&self.name)
            .expect("actor registration disappeared before startup");
        *actor = Some(mailbox.erase());
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.state.actors.lock().unwrap().remove(&self.name);
    }
}
