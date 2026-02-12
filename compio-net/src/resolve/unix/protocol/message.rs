use zerocopy::FromBytes;

use super::{header::RawHeader, record::Record};

pub struct Message<'a> {
    pub header: RawHeader,
    pub answers: Vec<Record<'a>>,
}

impl<'a> Message<'a> {
    pub fn read(buf: &'a [u8]) -> std::io::Result<Self> {
        let (header, _) = RawHeader::read_from_prefix(buf).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "buffer too short for header")
        })?;

        let mut idx = size_of::<RawHeader>();

        // Skip questions
        for _ in 0..header.qd_count.get() {
            idx += super::record::skip_name(buf, idx)?;
            if buf.len() < idx + 4 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "buffer too short skipping questions",
                ));
            }
            idx += 4; // qtype + qclass
        }

        let mut answers = Vec::with_capacity(header.an_count.get() as usize);
        for _ in 0..header.an_count.get() {
            let (record, new_idx) = Record::read(buf, idx)?;
            answers.push(record);
            idx = new_idx;
        }

        Ok(Self { header, answers })
    }
}
