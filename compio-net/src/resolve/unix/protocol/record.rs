use std::net::{Ipv4Addr, Ipv6Addr};

use zerocopy::{FromBytes, Immutable, KnownLayout, network_endian::*};

/// Record fixed fields (10 bytes, zero-copy).
#[derive(FromBytes, KnownLayout, Immutable, Debug, Clone, Copy)]
#[repr(C)]
struct RecordFields {
    rtype: U16,
    rclass: U16,
    ttl: U32,
    rdlength: U16,
}

pub struct Record<'a> {
    rtype: u16,
    ttl: u32,
    rdata: &'a [u8],
}

impl<'a> Record<'a> {
    pub(super) fn read(buf: &'a [u8], offset: usize) -> std::io::Result<(Self, usize)> {
        let mut idx = offset + skip_name(buf, offset)?;

        let fields_buf = buf.get(idx..idx + 10).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "buffer too short for record")
        })?;
        let fields = RecordFields::read_from_bytes(fields_buf).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "buffer too short for record")
        })?;
        idx += 10;

        let rdlength = fields.rdlength.get() as usize;
        let rdata = buf.get(idx..idx + rdlength).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "buffer too short for rdata")
        })?;
        idx += rdlength;

        Ok((
            Self {
                rtype: fields.rtype.get(),
                ttl: fields.ttl.get(),
                rdata,
            },
            idx,
        ))
    }

    pub fn ttl(&self) -> u32 {
        self.ttl
    }

    pub fn to_ip(&self) -> Option<std::net::IpAddr> {
        match self.rtype {
            1 => {
                let &bytes = <[u8; 4]>::ref_from_bytes(self.rdata).ok()?;
                Some(std::net::IpAddr::V4(Ipv4Addr::from(bytes)))
            }
            28 => {
                let &bytes = <[u8; 16]>::ref_from_bytes(self.rdata).ok()?;
                Some(std::net::IpAddr::V6(Ipv6Addr::from(bytes)))
            }
            _ => None,
        }
    }
}

/// Skip a DNS name at `offset`, returning bytes consumed.
pub(super) fn skip_name(buf: &[u8], offset: usize) -> std::io::Result<usize> {
    let mut idx = offset;
    let mut jumped = false;
    let mut end = 0usize;

    for _ in 0..256 {
        let &len = buf.get(idx).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "buffer too short for name")
        })?;

        if len == 0 {
            if !jumped {
                end = idx + 1;
            }
            return Ok(end - offset);
        }

        if (len & 0xC0) == 0xC0 {
            let &lo = buf.get(idx + 1).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "buffer too short for pointer",
                )
            })?;
            if !jumped {
                end = idx + 2;
            }
            idx = ((len as usize & 0x3F) << 8) | lo as usize;
            jumped = true;
            continue;
        }

        idx += 1 + len as usize;
        if idx > buf.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "buffer too short for label",
            ));
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "loop detected in name compression",
    ))
}
