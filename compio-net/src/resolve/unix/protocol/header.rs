use compio_buf::bytes::BufMut;
use zerocopy::{FromBytes, Immutable, KnownLayout, network_endian::*};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseCode {
    NoError,
    FormatError,
    ServerFailure,
    NameError,
    NotImplemented,
    Refused,
    Other(u8),
}

impl From<u8> for ResponseCode {
    fn from(code: u8) -> Self {
        match code {
            0 => Self::NoError,
            1 => Self::FormatError,
            2 => Self::ServerFailure,
            3 => Self::NameError,
            4 => Self::NotImplemented,
            5 => Self::Refused,
            c => Self::Other(c),
        }
    }
}

/// DNS header (12 bytes, zero-copy read).
#[derive(FromBytes, KnownLayout, Immutable, Debug, Clone, Copy)]
#[repr(C)]
pub struct RawHeader {
    pub id: U16,
    pub flags: U16,
    pub qd_count: U16,
    pub an_count: U16,
    pub ns_count: U16,
    pub ar_count: U16,
}

impl RawHeader {
    pub fn response_code(&self) -> ResponseCode {
        ResponseCode::from((self.flags.get() & 0xF) as u8)
    }

    pub fn is_response(&self) -> bool {
        (self.flags.get() & 0x8000) != 0
    }

    pub fn truncated(&self) -> bool {
        (self.flags.get() & 0x0200) != 0
    }
}

/// Write a standard DNS query header (RD=1, QDCOUNT=1).
pub fn write_query(id: u16, buf: &mut impl BufMut) {
    buf.put_slice(&id.to_be_bytes());
    buf.put_slice(&0x0100u16.to_be_bytes()); // flags: RD=1
    buf.put_slice(&1u16.to_be_bytes());      // qd_count
    buf.put_slice(&[0; 6]);                  // an/ns/ar = 0
}
