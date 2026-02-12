use compio_buf::bytes::BufMut;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryType {
    A    = 1,
    Aaaa = 28,
}

/// Write a DNS question section entry.
pub fn write_question(
    name: &str,
    qtype: QueryType,
    buf: &mut impl BufMut,
) -> std::io::Result<()> {
    for label in name.split('.') {
        if label.len() > 63 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "label too long",
            ));
        }
        buf.put_u8(label.len() as u8);
        buf.put_slice(label.as_bytes());
    }
    buf.put_u8(0);
    buf.put_slice(&(qtype as u16).to_be_bytes());
    buf.put_slice(&1u16.to_be_bytes()); // IN class
    Ok(())
}
