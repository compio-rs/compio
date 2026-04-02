use super::*;

#[derive(Debug)]
pub(in crate::sys) enum Extra {
    Poll(poll::Extra),
    IoUring(iour::Extra),
}

impl From<poll::Extra> for Extra {
    fn from(inner: poll::Extra) -> Self {
        Self::Poll(inner)
    }
}

impl From<iour::Extra> for Extra {
    fn from(inner: iour::Extra) -> Self {
        Self::IoUring(inner)
    }
}

#[allow(dead_code)]
impl crate::sys::Extra {
    pub(crate) fn is_iour(&self) -> bool {
        matches!(self.0, Extra::IoUring(_))
    }

    pub(in crate::sys) fn try_as_iour(&self) -> Option<&iour::Extra> {
        if let Extra::IoUring(extra) = &self.0 {
            Some(extra)
        } else {
            None
        }
    }

    pub(in crate::sys) fn try_as_iour_mut(&mut self) -> Option<&mut iour::Extra> {
        if let Extra::IoUring(extra) = &mut self.0 {
            Some(extra)
        } else {
            None
        }
    }

    pub(in crate::sys) fn try_as_poll(&self) -> Option<&poll::Extra> {
        if let Extra::Poll(extra) = &self.0 {
            Some(extra)
        } else {
            None
        }
    }

    pub(in crate::sys) fn try_as_poll_mut(&mut self) -> Option<&mut poll::Extra> {
        if let Extra::Poll(extra) = &mut self.0 {
            Some(extra)
        } else {
            None
        }
    }
}
