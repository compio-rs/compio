use compio_buf::BufResult;
use compio_driver::ring_mapped_buffers::RingMappedBuffer;
use compio_util::ring_mapped_buffers::RingMappedBuffers;

use crate::{TcpStream, UdpSocket};

pub trait ReadExt {
    async fn read_with_ring_mapped_buffers(
        &self,
        buffers: &RingMappedBuffers,
    ) -> BufResult<RingMappedBuffer, ()>;
}

#[cfg(feature = "runtime")]
impl ReadExt for TcpStream {
    async fn read_with_ring_mapped_buffers(
        &self,
        buffers: &RingMappedBuffers,
    ) -> BufResult<RingMappedBuffer, ()> {
        self.inner
            .recv_with_ring_mapped_buffers(buffers.as_raw().clone())
            .await
            .map_buffer(|_| ())
    }
}

#[cfg(feature = "runtime")]
impl ReadExt for UdpSocket {
    async fn read_with_ring_mapped_buffers(
        &self,
        buffers: &RingMappedBuffers,
    ) -> BufResult<RingMappedBuffer, ()> {
        self.inner
            .recv_with_ring_mapped_buffers(buffers.as_raw().clone())
            .await
            .map_buffer(|_| ())
    }
}

#[cfg(feature = "runtime")]
#[cfg(test)]
mod tests {
    use compio_driver::ring_mapped_buffers::Builder;
    use compio_io::AsyncWrite;
    use compio_runtime::{block_on, spawn};

    use super::*;
    use crate::TcpListener;

    #[test]
    fn test_tcp_recv_with_register_buffers() {
        block_on(async move {
            let listener = TcpListener::bind("[::1]:0").unwrap();
            let addr = listener.local_addr().unwrap();

            let task = spawn(async move { TcpStream::connect(addr).await.unwrap() });

            let mut stream1 = listener.accept().await.unwrap().0;
            let stream2 = task.await;

            stream1.write(&b"test"[..]).await.unwrap();

            let builder = Builder::new(1).buf_len(4096).buf_cnt(1);
            let buffers = RingMappedBuffers::build(builder).unwrap();

            let (buf, _buffers) = stream2
                .read_with_ring_mapped_buffers(&buffers)
                .await
                .unwrap();

            assert_eq!(buf.as_ref(), b"test");
        })
    }

    #[test]
    fn test_udp_recv_with_register_buffers() {
        block_on(async move {
            let mut udp_socket1 = UdpSocket::bind("[::1]:0").unwrap();
            let addr = udp_socket1.local_addr().unwrap();

            let udp_socket2 = UdpSocket::bind("[::]:0").unwrap();
            udp_socket2.connect(addr).unwrap();
            let addr = udp_socket2.local_addr().unwrap();

            udp_socket1.send_to(&b"test"[..], addr).await.unwrap();

            let builder = Builder::new(1).buf_len(4096).buf_cnt(1);
            let buffers = RingMappedBuffers::build(builder).unwrap();

            let (buf, _buffers) = udp_socket2
                .read_with_ring_mapped_buffers(&buffers)
                .await
                .unwrap();

            assert_eq!(buf.as_ref(), b"test");
        })
    }
}
