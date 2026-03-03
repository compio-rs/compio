struct Ancillary<const N: usize> {
    inner: [u8; N],
    len: usize,
    #[cfg(unix)]
    _align: [libc::cmsghdr; 0],
    #[cfg(windows)]
    _align: [WinSock::CMSGHDR; 0],
}

impl<const N: usize> Ancillary<N> {
    fn new() -> Self {
        Self {
            inner: [0u8; N],
            len: 0,
            _align: [],
        }
    }
}

impl<const N: usize> IoBuf for Ancillary<N> {
    fn as_init(&self) -> &[u8] {
        &self.inner[..self.len]
    }
}

impl<const N: usize> SetLen for Ancillary<N> {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= N);
        self.len = len;
    }
}

impl<const N: usize> IoBufMut for Ancillary<N> {
    fn as_uninit(&mut self) -> &mut [std::mem::MaybeUninit<u8>] {
        self.inner.as_uninit()
    }
}

impl<const N: usize> Deref for Ancillary<N> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.inner[0..self.len]
    }
}

impl<const N: usize> DerefMut for Ancillary<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner[0..self.len]
    }
}
