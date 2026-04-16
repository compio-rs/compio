use std::{collections::VecDeque, io};

use super::*;
use crate::buffer_pool::BufPtr;

#[derive(Debug)]
pub(in crate::sys) struct BufControl {
    queue: VecDeque<u16>,
}

impl BufControl {
    pub fn new(bufs: &[Slot]) -> Self {
        assert!(bufs.len() < u16::MAX as _);
        Self {
            queue: bufs.iter().enumerate().map(|(id, _)| id as u16).collect(),
        }
    }

    #[allow(dead_code)]
    pub unsafe fn release(&mut self, _: &mut crate::Driver) -> io::Result<()> {
        Ok(())
    }

    pub fn pop(&mut self) -> io::Result<u16> {
        self.queue
            .pop_front()
            .ok_or_else(|| io::Error::other("buffer ring has no available buffer"))
    }

    pub unsafe fn reset(&mut self, buffer_id: u16, _: BufPtr, _: u32) {
        self.queue.push_back(buffer_id);
    }
}
