#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    marker::PhantomPinned,
    net::Shutdown,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    pin::Pin,
    ptr::{null, null_mut},
    task::Poll,
};

use aligned_array::{Aligned, A8};
use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use socket2::SockAddr;
use widestring::U16CString;
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE,
            ERROR_IO_PENDING, ERROR_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
            ERROR_SHARING_VIOLATION, FILETIME, INVALID_HANDLE_VALUE,
        },
        Networking::WinSock::{
            closesocket, setsockopt, shutdown, socklen_t, WSAIoctl, WSARecv, WSARecvFrom, WSASend,
            WSASendTo, LPFN_ACCEPTEX, LPFN_CONNECTEX, LPFN_GETACCEPTEXSOCKADDRS, SD_BOTH,
            SD_RECEIVE, SD_SEND, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKADDR_STORAGE,
            SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSAID_ACCEPTEX,
            WSAID_CONNECTEX, WSAID_GETACCEPTEXSOCKADDRS,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{
            CreateFileW, FileAttributeTagInfo, FindClose, FindFirstFileW, FlushFileBuffers,
            GetFileInformationByHandle, GetFileInformationByHandleEx, ReadFile, WriteFile,
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
            FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_MODE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING, WIN32_FIND_DATAW,
        },
        System::{
            Pipes::ConnectNamedPipe,
            IO::{CancelIoEx, OVERLAPPED},
        },
    },
};

use crate::{op::*, syscall, OpCode, RawFd};

#[inline]
fn winapi_result(transferred: u32) -> Poll<io::Result<usize>> {
    let error = unsafe { GetLastError() };
    assert_ne!(error, 0);
    match error {
        ERROR_IO_PENDING => Poll::Pending,
        ERROR_IO_INCOMPLETE | ERROR_HANDLE_EOF | ERROR_PIPE_CONNECTED | ERROR_NO_DATA => {
            Poll::Ready(Ok(transferred as _))
        }
        _ => Poll::Ready(Err(io::Error::from_raw_os_error(error as _))),
    }
}

#[inline]
fn win32_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res == 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

#[inline]
fn winsock_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res != 0 {
        winapi_result(transferred)
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

#[inline]
fn cancel(handle: RawFd, optr: *mut OVERLAPPED) -> io::Result<()> {
    match syscall!(BOOL, CancelIoEx(handle as _, optr)) {
        Ok(_) => Ok(()),
        Err(e) => {
            if e.raw_os_error() == Some(ERROR_NOT_FOUND as _) {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

fn get_wsa_fn<F>(handle: RawFd, fguid: GUID) -> io::Result<Option<F>> {
    let mut fptr = None;
    let mut returned = 0;
    syscall!(
        SOCKET,
        WSAIoctl(
            handle as _,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            std::ptr::addr_of!(fguid).cast(),
            std::mem::size_of_val(&fguid) as _,
            std::ptr::addr_of_mut!(fptr).cast(),
            std::mem::size_of::<F>() as _,
            &mut returned,
            null_mut(),
            None,
        )
    )?;
    Ok(fptr)
}

impl<
    D: std::marker::Send + 'static,
    F: (FnOnce() -> BufResult<usize, D>) + std::marker::Send + std::marker::Sync + 'static,
> OpCode for Asyncify<F, D>
{
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        // Safety: self won't be moved
        let this = self.get_unchecked_mut();
        let f = this
            .f
            .take()
            .expect("the operate method could only be called once");
        let BufResult(res, data) = f();
        this.data = Some(data);
        Poll::Ready(res)
    }
}

/// Open or create a file with flags and mode.
pub struct OpenFile {
    pub(crate) path: U16CString,
    pub(crate) access_mode: u32,
    pub(crate) share_mode: FILE_SHARE_MODE,
    pub(crate) security_attributes: *const SECURITY_ATTRIBUTES,
    pub(crate) creation_mode: FILE_CREATION_DISPOSITION,
    pub(crate) flags_and_attributes: FILE_FLAGS_AND_ATTRIBUTES,
    pub(crate) error_code: u32,
}

impl OpenFile {
    /// Create [`OpenFile`].
    pub fn new(
        path: U16CString,
        access_mode: u32,
        share_mode: FILE_SHARE_MODE,
        security_attributes: *const SECURITY_ATTRIBUTES,
        creation_mode: FILE_CREATION_DISPOSITION,
        flags_and_attributes: FILE_FLAGS_AND_ATTRIBUTES,
    ) -> Self {
        Self {
            path,
            access_mode,
            share_mode,
            security_attributes,
            creation_mode,
            flags_and_attributes,
            error_code: 0,
        }
    }

    /// The result of [`GetLastError`]. It may not be 0 even if the operation is
    /// successful.
    pub fn last_os_error(&self) -> u32 {
        self.error_code
    }
}

impl OpCode for OpenFile {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(mut self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let handle = CreateFileW(
            self.path.as_ptr(),
            self.access_mode,
            self.share_mode,
            self.security_attributes,
            self.creation_mode,
            self.flags_and_attributes,
            0,
        );
        self.error_code = GetLastError();
        if handle == INVALID_HANDLE_VALUE {
            Poll::Ready(Err(io::Error::from_raw_os_error(self.error_code as _)))
        } else {
            Poll::Ready(Ok(handle as _))
        }
    }
}

impl OpCode for CloseFile {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(BOOL, CloseHandle(self.fd as _))? as _))
    }
}

/// A mixture of [`BY_HANDLE_FILE_INFORMATION`], [`FILE_ATTRIBUTE_TAG_INFO`] and
/// [`WIN32_FIND_DATAW`]. The field names follows Hungarian case, to make it
/// look like Windows API.
#[derive(Default, Clone)]
#[allow(non_snake_case, missing_docs)]
pub struct FileMetadata {
    pub dwFileAttributes: u32,
    pub ftCreationTime: u64,
    pub ftLastAccessTime: u64,
    pub ftLastWriteTime: u64,
    pub nFileSize: u64,
    pub dwReparseTag: u32,
    pub dwVolumeSerialNumber: Option<u32>,
    pub nNumberOfLinks: Option<u32>,
    pub nFileIndex: Option<u64>,
}

impl FileMetadata {
    fn is_reparse_point(&self) -> bool {
        self.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

const fn create_u64(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | (low as u64)
}

const fn filetime_u64(t: FILETIME) -> u64 {
    create_u64(t.dwHighDateTime, t.dwLowDateTime)
}

impl From<BY_HANDLE_FILE_INFORMATION> for FileMetadata {
    fn from(value: BY_HANDLE_FILE_INFORMATION) -> Self {
        Self {
            dwFileAttributes: value.dwFileAttributes,
            ftCreationTime: filetime_u64(value.ftCreationTime),
            ftLastAccessTime: filetime_u64(value.ftLastAccessTime),
            ftLastWriteTime: filetime_u64(value.ftLastWriteTime),
            nFileSize: create_u64(value.nFileSizeHigh, value.nFileSizeLow),
            dwReparseTag: 0,
            dwVolumeSerialNumber: Some(value.dwVolumeSerialNumber),
            nNumberOfLinks: Some(value.nNumberOfLinks),
            nFileIndex: Some(create_u64(value.nFileIndexHigh, value.nFileIndexLow)),
        }
    }
}

impl From<WIN32_FIND_DATAW> for FileMetadata {
    fn from(value: WIN32_FIND_DATAW) -> Self {
        let mut this = Self {
            dwFileAttributes: value.dwFileAttributes,
            ftCreationTime: filetime_u64(value.ftCreationTime),
            ftLastAccessTime: filetime_u64(value.ftLastAccessTime),
            ftLastWriteTime: filetime_u64(value.ftLastWriteTime),
            nFileSize: create_u64(value.nFileSizeHigh, value.nFileSizeLow),
            dwReparseTag: 0,
            dwVolumeSerialNumber: None,
            nNumberOfLinks: None,
            nFileIndex: None,
        };
        if this.is_reparse_point() {
            this.dwReparseTag = value.dwReserved0;
        }
        this
    }
}

/// Get metadata of an opened file.
pub struct FileStat {
    pub(crate) fd: RawFd,
    pub(crate) stat: FileMetadata,
}

impl FileStat {
    /// Create [`FileStat`].
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            stat: Default::default(),
        }
    }
}

impl OpCode for FileStat {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(mut self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let mut stat = unsafe { std::mem::zeroed() };
        syscall!(BOOL, GetFileInformationByHandle(self.fd as _, &mut stat))?;
        self.stat = stat.into();
        if self.stat.is_reparse_point() {
            let mut tag: FILE_ATTRIBUTE_TAG_INFO = std::mem::zeroed();
            syscall!(
                BOOL,
                GetFileInformationByHandleEx(
                    self.fd as _,
                    FileAttributeTagInfo,
                    &mut tag as *mut _ as _,
                    std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as _
                )
            )?;
            debug_assert_eq!(self.stat.dwFileAttributes, tag.FileAttributes);
            self.stat.dwReparseTag = tag.ReparseTag;
        }
        Poll::Ready(Ok(0))
    }

    unsafe fn cancel(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> io::Result<()> {
        Ok(())
    }
}

impl IntoInner for FileStat {
    type Inner = FileMetadata;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

/// Get metadata from path.
pub struct PathStat {
    pub(crate) path: U16CString,
    pub(crate) follow_symlink: bool,
    pub(crate) stat: FileMetadata,
}

impl PathStat {
    /// Create [`PathStat`].
    pub fn new(path: U16CString, follow_symlink: bool) -> Self {
        Self {
            path,
            follow_symlink,
            stat: Default::default(),
        }
    }

    unsafe fn open_and_stat(&self, optr: *mut OVERLAPPED) -> io::Result<FileMetadata> {
        let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
        if !self.follow_symlink {
            flags |= FILE_FLAG_OPEN_REPARSE_POINT;
        }
        let handle = syscall!(
            HANDLE,
            CreateFileW(
                self.path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                flags,
                0
            )
        )?;
        let handle = OwnedHandle::from_raw_handle(handle as _);
        let mut op = FileStat::new(handle.as_raw_handle());
        let op_pin = std::pin::Pin::new(&mut op);
        let res = op_pin.operate(optr);
        if let Poll::Ready(res) = res {
            res.map(|_| op.into_inner())
        } else {
            unreachable!("FileStat could not return Poll::Pending")
        }
    }
}

impl OpCode for PathStat {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(mut self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = match self.open_and_stat(optr) {
            Ok(stat) => {
                self.stat = stat;
                Ok(0)
            }
            Err(e)
                if [
                    Some(ERROR_SHARING_VIOLATION as _),
                    Some(ERROR_ACCESS_DENIED as _),
                ]
                .contains(&e.raw_os_error()) =>
            {
                let mut wfd: WIN32_FIND_DATAW = std::mem::zeroed();
                let handle = syscall!(HANDLE, FindFirstFileW(self.path.as_ptr(), &mut wfd))?;
                FindClose(handle);
                self.stat = wfd.into();
                let is_reparse = self.stat.is_reparse_point();
                let surrogate = self.stat.dwReparseTag & 0x20000000 != 0;
                if self.follow_symlink && is_reparse && surrogate {
                    Err(e)
                } else {
                    Ok(0)
                }
            }
            Err(e) => Err(e),
        };
        Poll::Ready(res)
    }

    unsafe fn cancel(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> io::Result<()> {
        Ok(())
    }
}

impl IntoInner for PathStat {
    type Inner = FileMetadata;

    fn into_inner(self) -> Self::Inner {
        self.stat
    }
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let fd = self.fd as _;
        let slice = self.get_unchecked_mut().buffer.as_mut_slice();
        let mut transferred = 0;
        let res = ReadFile(
            fd,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = WriteFile(
            self.fd as _,
            slice.as_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

impl OpCode for Sync {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(BOOL, FlushFileBuffers(self.fd as _))? as _))
    }
}

impl OpCode for ShutdownSocket {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let how = match self.how {
            Shutdown::Write => SD_SEND,
            Shutdown::Read => SD_RECEIVE,
            Shutdown::Both => SD_BOTH,
        };
        Poll::Ready(Ok(syscall!(SOCKET, shutdown(self.fd as _, how))? as _))
    }
}

impl OpCode for CloseSocket {
    fn is_overlapped(&self) -> bool {
        false
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(syscall!(SOCKET, closesocket(self.fd as _))? as _))
    }
}

static ACCEPT_EX: OnceLock<LPFN_ACCEPTEX> = OnceLock::new();
static GET_ADDRS: OnceLock<LPFN_GETACCEPTEXSOCKADDRS> = OnceLock::new();

const ACCEPT_ADDR_BUFFER_SIZE: usize = std::mem::size_of::<SOCKADDR_STORAGE>() + 16;
const ACCEPT_BUFFER_SIZE: usize = ACCEPT_ADDR_BUFFER_SIZE * 2;

/// Accept a connection.
pub struct Accept {
    pub(crate) fd: RawFd,
    pub(crate) accept_fd: RawFd,
    pub(crate) buffer: Aligned<A8, [u8; ACCEPT_BUFFER_SIZE]>,
    _p: PhantomPinned,
}

impl Accept {
    /// Create [`Accept`]. `accept_fd` should not be bound.
    pub fn new(fd: RawFd, accept_fd: RawFd) -> Self {
        Self {
            fd,
            accept_fd,
            buffer: unsafe { std::mem::zeroed() },
            _p: PhantomPinned,
        }
    }

    /// Update accept context.
    pub fn update_context(&self) -> io::Result<()> {
        syscall!(
            SOCKET,
            setsockopt(
                self.accept_fd as _,
                SOL_SOCKET,
                SO_UPDATE_ACCEPT_CONTEXT,
                &self.fd as *const _ as _,
                std::mem::size_of_val(&self.fd) as _,
            )
        )?;
        Ok(())
    }

    /// Get the remote address from the inner buffer.
    pub fn into_addr(self) -> io::Result<SockAddr> {
        let get_addrs_fn = GET_ADDRS
            .get_or_try_init(|| get_wsa_fn(self.fd, WSAID_GETACCEPTEXSOCKADDRS))?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "cannot retrieve GetAcceptExSockAddrs",
                )
            })?;
        let mut local_addr: *mut SOCKADDR = null_mut();
        let mut local_addr_len = 0;
        let mut remote_addr: *mut SOCKADDR = null_mut();
        let mut remote_addr_len = 0;
        unsafe {
            get_addrs_fn(
                &self.buffer as *const _ as *const _,
                0,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                ACCEPT_ADDR_BUFFER_SIZE as _,
                &mut local_addr,
                &mut local_addr_len,
                &mut remote_addr,
                &mut remote_addr_len,
            );
        }
        Ok(unsafe { SockAddr::new(*remote_addr.cast::<SOCKADDR_STORAGE>(), remote_addr_len) })
    }
}

impl OpCode for Accept {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let accept_fn = ACCEPT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd, WSAID_ACCEPTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve AcceptEx")
            })?;
        let mut received = 0;
        let res = accept_fn(
            self.fd as _,
            self.accept_fd as _,
            self.get_unchecked_mut().buffer.as_mut_ptr() as _,
            0,
            ACCEPT_ADDR_BUFFER_SIZE as _,
            ACCEPT_ADDR_BUFFER_SIZE as _,
            &mut received,
            optr,
        );
        win32_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

static CONNECT_EX: OnceLock<LPFN_CONNECTEX> = OnceLock::new();

impl Connect {
    /// Update connect context.
    pub fn update_context(&self) -> io::Result<()> {
        syscall!(
            SOCKET,
            setsockopt(
                self.fd as _,
                SOL_SOCKET,
                SO_UPDATE_CONNECT_CONTEXT,
                null(),
                0,
            )
        )?;
        Ok(())
    }
}

impl OpCode for Connect {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let connect_fn = CONNECT_EX
            .get_or_try_init(|| get_wsa_fn(self.fd, WSAID_CONNECTEX))?
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::Unsupported, "cannot retrieve ConnectEx")
            })?;
        let mut sent = 0;
        let res = connect_fn(
            self.fd as _,
            self.addr.as_ptr(),
            self.addr.len(),
            null(),
            0,
            &mut sent,
            optr,
        );
        win32_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data from remote.
pub struct Recv<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBufMut> Recv<T> {
    /// Create [`Recv`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut> IntoInner for Recv<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBufMut> OpCode for Recv<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd as _;
        let slice = self.get_unchecked_mut().buffer.as_mut_slice();
        let mut transferred = 0;
        let res = ReadFile(
            fd,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data from remote into vectored buffer.
pub struct RecvVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut> RecvVectored<T> {
    /// Create [`RecvVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvVectored<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let fd = self.fd;
        let slices = self.get_unchecked_mut().buffer.as_io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecv(
            fd as _,
            slices.as_ptr() as _,
            slices.len() as _,
            &mut received,
            &mut flags,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to remote.
pub struct Send<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoBuf> Send<T> {
    /// Create [`Send`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf> IntoInner for Send<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> OpCode for Send<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slice = self.buffer.as_slice();
        let mut transferred = 0;
        let res = WriteFile(
            self.fd as _,
            slice.as_ptr() as _,
            slice.len() as _,
            &mut transferred,
            optr,
        );
        win32_result(res, transferred)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to remote from vectored buffer.
pub struct SendVectored<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf> SendVectored<T> {
    /// Create [`SendVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf> IntoInner for SendVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf> OpCode for SendVectored<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let slices = self.buffer.as_io_slices();
        let mut sent = 0;
        let res = WSASend(
            self.fd as _,
            slices.as_ptr() as _,
            slices.len() as _,
            &mut sent,
            0,
            optr,
            None,
        );
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data and source address.
pub struct RecvFrom<T: IoBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoBufMut> RecvFrom<T> {
    /// Create [`RecvFrom`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBufMut> IntoInner for RecvFrom<T> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoBufMut> OpCode for RecvFrom<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let fd = this.fd;
        let buffer = this.buffer.as_io_slice_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            fd as _,
            &buffer as *const _ as _,
            1,
            &mut received,
            &mut flags,
            &mut this.addr as *mut _ as *mut SOCKADDR,
            &mut this.addr_len,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Receive data and source address into vectored buffer.
pub struct RecvFromVectored<T: IoVectoredBufMut> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SOCKADDR_STORAGE,
    pub(crate) addr_len: socklen_t,
    _p: PhantomPinned,
}

impl<T: IoVectoredBufMut> RecvFromVectored<T> {
    /// Create [`RecvFromVectored`].
    pub fn new(fd: RawFd, buffer: T) -> Self {
        Self {
            fd,
            buffer,
            addr: unsafe { std::mem::zeroed() },
            addr_len: std::mem::size_of::<SOCKADDR_STORAGE>() as _,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBufMut> IntoInner for RecvFromVectored<T> {
    type Inner = (T, SOCKADDR_STORAGE, socklen_t);

    fn into_inner(self) -> Self::Inner {
        (self.buffer, self.addr, self.addr_len)
    }
}

impl<T: IoVectoredBufMut> OpCode for RecvFromVectored<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let this = self.get_unchecked_mut();
        let fd = this.fd;
        let buffer = this.buffer.as_io_slices_mut();
        let mut flags = 0;
        let mut received = 0;
        let res = WSARecvFrom(
            fd as _,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut received,
            &mut flags,
            &mut this.addr as *mut _ as *mut SOCKADDR,
            &mut this.addr_len,
            optr,
            None,
        );
        winsock_result(res, received)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to specified address.
pub struct SendTo<T: IoBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoBuf> SendTo<T> {
    /// Create [`SendTo`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoBuf> IntoInner for SendTo<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoBuf> OpCode for SendTo<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slice();
        let mut sent = 0;
        let res = WSASendTo(
            self.fd as _,
            &buffer as *const _ as _,
            1,
            &mut sent,
            0,
            self.addr.as_ptr(),
            self.addr.len(),
            optr,
            None,
        );
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Send data to specified address from vectored buffer.
pub struct SendToVectored<T: IoVectoredBuf> {
    pub(crate) fd: RawFd,
    pub(crate) buffer: T,
    pub(crate) addr: SockAddr,
    _p: PhantomPinned,
}

impl<T: IoVectoredBuf> SendToVectored<T> {
    /// Create [`SendToVectored`].
    pub fn new(fd: RawFd, buffer: T, addr: SockAddr) -> Self {
        Self {
            fd,
            buffer,
            addr,
            _p: PhantomPinned,
        }
    }
}

impl<T: IoVectoredBuf> IntoInner for SendToVectored<T> {
    type Inner = T;

    fn into_inner(self) -> Self::Inner {
        self.buffer
    }
}

impl<T: IoVectoredBuf> OpCode for SendToVectored<T> {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let buffer = self.buffer.as_io_slices();
        let mut sent = 0;
        let res = WSASendTo(
            self.fd as _,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut sent,
            0,
            self.addr.as_ptr(),
            self.addr.len(),
            optr,
            None,
        );
        winsock_result(res, sent)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}

/// Connect a named pipe server.
pub struct ConnectNamedPipe {
    pub(crate) fd: RawFd,
}

impl ConnectNamedPipe {
    /// Create [`ConnectNamedPipe`](struct@ConnectNamedPipe).
    pub fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl OpCode for ConnectNamedPipe {
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let res = ConnectNamedPipe(self.fd as _, optr);
        win32_result(res, 0)
    }

    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        cancel(self.fd, optr)
    }
}
