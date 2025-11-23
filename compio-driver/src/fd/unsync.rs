pub use std::rc::Rc as RefPtr;
use std::{
    cell::{Cell, RefCell},
    sync::atomic::Ordering,
    task::Waker,
};

#[derive(Debug)]
pub struct WaitFlag(Cell<bool>);

impl WaitFlag {
    pub fn new(v: bool) -> Self {
        Self(Cell::new(v))
    }

    pub fn load(&self, _ordering: Ordering) -> bool {
        self.0.get()
    }

    pub fn swap(&self, v: bool, _ordering: Ordering) -> bool {
        let old = self.0.get();
        self.0.set(v);
        old
    }
}

#[derive(Debug)]
pub struct WakerRegistry {
    waker: RefCell<Option<Waker>>,
}

impl WakerRegistry {
    pub fn new() -> Self {
        Self {
            waker: RefCell::new(None),
        }
    }

    pub fn register(&self, waker: &Waker) {
        self.waker.borrow_mut().replace(waker.clone());
    }

    pub fn wake(&self) {
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake()
        }
    }
}
