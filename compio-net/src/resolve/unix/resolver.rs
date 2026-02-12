use std::{
    io,
    net::{IpAddr, SocketAddr},
};

use compio_buf::{bytes::BufMut, BufResult};
use compio_io::{AsyncReadExt, AsyncWriteExt};
use once_cell::sync::OnceCell;

use super::{
    config::ResolvConf,
    protocol::{Message, QueryType, ResponseCode, write_query, write_question},
};

static RESOLV_CONF: OnceCell<ResolvConf> = OnceCell::new();
static HOSTS: OnceCell<String> = OnceCell::new();


pub struct AsyncResolver {
    resolv_conf: &'static ResolvConf,
}

impl AsyncResolver {
    pub fn new() -> io::Result<Self> {
        let resolv_conf = RESOLV_CONF.get_or_try_init(ResolvConf::load)?;
        HOSTS.get_or_try_init(|| {
            std::fs::read_to_string("/etc/hosts").or_else(|_| Ok::<_, io::Error>(String::new()))
        })?;
        Ok(Self { resolv_conf })
    }

    pub async fn lookup(&self, name: &str) -> io::Result<std::vec::IntoIter<SocketAddr>> {
        // /etc/hosts
        if let Some(content) = HOSTS.get()
            && let Some(addr) = parse_hosts(content, name)
        {
            return Ok(vec![SocketAddr::new(addr, 0)].into_iter());
        }

        // IP literal
        if let Ok(ip) = name.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, 0)].into_iter());
        }

        for name in self.build_search_list(name) {
            // Check cache first
            #[cfg(feature = "dns-cache")]
            if let Some(addrs) = crate::resolve::DNS_CACHE.get(&name).await {
                return Ok(addrs
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, 0))
                    .collect::<Vec<_>>()
                    .into_iter());
            }

            if let Ok(result) = self.query(&name).await {
                #[cfg(feature = "dns-cache")]
                {
                    let ttl = if result.addrs.is_empty() {
                        60
                    } else {
                        result.min_ttl
                    };
                    crate::resolve::DNS_CACHE
                        .insert(name, result.addrs.clone(), ttl)
                        .await;
                }

                if !result.addrs.is_empty() {
                    return Ok(result
                        .addrs
                        .into_iter()
                        .map(|ip| SocketAddr::new(ip, 0))
                        .collect::<Vec<_>>()
                        .into_iter());
                }
            }
        }

        Err(io::Error::other("failed to resolve"))
    }

    fn build_search_list(&self, name: &str) -> Vec<String> {
        let mut names = Vec::new();
        if name.ends_with('.') {
            names.push(name.trim_end_matches('.').to_string());
            return names;
        }

        let ndots = name.bytes().filter(|&b| b == b'.').count();
        if ndots >= self.resolv_conf.ndots as usize {
            names.push(name.to_string());
        }
        for domain in &self.resolv_conf.search {
            names.push(format!("{name}.{domain}"));
        }
        if ndots < self.resolv_conf.ndots as usize {
            names.push(name.to_string());
        }
        names
    }

    async fn query(&self, name: &str) -> io::Result<QueryResult> {
        let futures: Vec<_> = self
            .resolv_conf
            .nameservers
            .iter()
            .map(|ns| Box::pin(self.query_ns_all(name, *ns)))
            .collect();

        if futures.is_empty() {
            return Ok(QueryResult::empty());
        }

        use futures_util::future::select_all;

        let mut remaining = futures;
        while !remaining.is_empty() {
            let (result, _, rest) = select_all(remaining).await;
            remaining = rest;
            if let Ok(r) = result
                && !r.addrs.is_empty()
            {
                return Ok(r);
            }
        }

        Ok(QueryResult::empty())
    }

    async fn query_ns_all(&self, name: &str, ns: SocketAddr) -> io::Result<QueryResult> {
        let r = self.query_ns(name, ns, QueryType::A).await?;
        if !r.addrs.is_empty() {
            return Ok(r);
        }
        self.query_ns(name, ns, QueryType::Aaaa).await
    }

    async fn query_ns(
        &self,
        name: &str,
        ns: SocketAddr,
        qtype: QueryType,
    ) -> io::Result<QueryResult> {
        let id = fastrand::u16(..);
        let mut buf = Vec::with_capacity(512);
        write_query(id, &mut buf);
        write_question(name, qtype, &mut buf)?;

        let socket = crate::UdpSocket::bind(&SocketAddr::from(([0, 0, 0, 0], 0))).await?;
        socket.connect(ns).await?;

        let mut recv_buf = Vec::with_capacity(512);
        for _ in 0..self.resolv_conf.attempts {
            let BufResult(res, b) = socket.send(buf).await;
            buf = b;
            res?;

            let BufResult(res, b) =
                compio_runtime::time::timeout(self.resolv_conf.timeout, socket.recv(recv_buf))
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "recv timed out"))?;
            recv_buf = b;
            let n = res?;

            if n < 12 {
                continue;
            }

            if let Ok(msg) = Message::read(&recv_buf[..n])
                && msg.header.id.get() == id
                && msg.header.is_response()
            {
                if msg.header.truncated() {
                    return self.query_ns_tcp(name, ns, qtype).await;
                }
                match msg.header.response_code() {
                    ResponseCode::NoError => return Ok(QueryResult::from_answers(&msg.answers)),
                    ResponseCode::NameError => return Ok(QueryResult::empty()),
                    _ => {}
                }
            }
        }

        Err(io::Error::new(io::ErrorKind::TimedOut, "query timed out"))
    }

    async fn query_ns_tcp(
        &self,
        name: &str,
        ns: SocketAddr,
        qtype: QueryType,
    ) -> io::Result<QueryResult> {
        let timeout = self.resolv_conf.timeout;

        let id = fastrand::u16(..);
        let mut buf = Vec::with_capacity(514);
        buf.put_u16(0); // length placeholder
        write_query(id, &mut buf);
        write_question(name, qtype, &mut buf)?;
        let len = (buf.len() - 2) as u16;
        buf[0..2].copy_from_slice(&len.to_be_bytes());

        let mut socket = compio_runtime::time::timeout(timeout, crate::TcpStream::connect(ns))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP connect timed out"))?
            ?;

        let BufResult(res, _) =
            compio_runtime::time::timeout(timeout, socket.write_all(buf))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP write timed out"))?;
        res?;

        let BufResult(res, len_buf) =
            compio_runtime::time::timeout(timeout, socket.read_exact([0u8; 2]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP read timed out"))?;
        res?;
        let resp_len = u16::from_be_bytes(len_buf) as usize;

        let BufResult(res, recv_buf) =
            compio_runtime::time::timeout(timeout, socket.read_exact(vec![0u8; resp_len]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TCP read timed out"))?;
        res?;

        let msg = Message::read(&recv_buf)?;
        if msg.header.id.get() != id {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ID mismatch"));
        }

        match msg.header.response_code() {
            ResponseCode::NoError => Ok(QueryResult::from_answers(&msg.answers)),
            ResponseCode::NameError => Ok(QueryResult::empty()),
            _ => Err(io::Error::other("DNS server returned error")),
        }
    }
}

/// Holds resolved IPs together with the minimum TTL from the answer section.
struct QueryResult {
    addrs: Vec<IpAddr>,
    min_ttl: u32,
}

impl QueryResult {
    fn empty() -> Self {
        Self {
            addrs: vec![],
            min_ttl: 0,
        }
    }

    fn from_answers(answers: &[super::protocol::Record<'_>]) -> Self {
        let mut addrs = Vec::new();
        let mut min_ttl = u32::MAX;
        for r in answers {
            if let Some(ip) = r.to_ip() {
                addrs.push(ip);
                min_ttl = min_ttl.min(r.ttl());
            }
        }
        if addrs.is_empty() {
            min_ttl = 0;
        }
        Self { addrs, min_ttl }
    }
}

fn parse_hosts(content: &str, name: &str) -> Option<IpAddr> {
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let Some(addr_str) = parts.next()
            && let Ok(addr) = addr_str.parse::<IpAddr>()
        {
            for host in parts {
                if host.eq_ignore_ascii_case(name) {
                    return Some(addr);
                }
            }
        }
    }
    None
}
