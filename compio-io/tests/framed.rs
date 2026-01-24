use std::{
    io::{self, Cursor},
    rc::Rc,
};

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{
    AsyncRead, AsyncReadAt, AsyncWrite, AsyncWriteAt,
    framed::{Framed, codec::serde_json::SerdeJsonCodec, frame::LengthDelimited},
};
use futures_util::{SinkExt, StreamExt, lock::Mutex};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Test {
    foo: String,
    bar: usize,
}

struct InMemoryPipe(Cursor<Rc<Mutex<Vec<u8>>>>);

impl AsyncRead for InMemoryPipe {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        let BufResult(res, buf) = self
            .0
            .get_ref()
            .lock()
            .await
            .read_at(buf, self.0.position())
            .await;
        match res {
            Ok(len) => {
                self.0.set_position(self.0.position() + len as u64);
                BufResult(Ok(len), buf)
            }
            Err(_) => BufResult(res, buf),
        }
    }
}

impl AsyncWrite for InMemoryPipe {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let BufResult(res, buf) = self
            .0
            .get_ref()
            .lock()
            .await
            .write_at(buf, self.0.position())
            .await;
        match res {
            Ok(len) => {
                self.0.set_position(self.0.position() + len as u64);
                BufResult(Ok(len), buf)
            }
            Err(_) => BufResult(res, buf),
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        self.0.get_ref().lock().await.flush().await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.get_ref().lock().await.shutdown().await
    }
}

#[compio_macros::test]
async fn test_framed() {
    let codec = SerdeJsonCodec::new();
    let framer = LengthDelimited::new();
    let buf = Rc::new(Mutex::new(vec![]));
    let ins = buf.clone();
    let r = InMemoryPipe(Cursor::new(buf.clone()));
    let w = InMemoryPipe(Cursor::new(buf));
    let mut framed = Framed::symmetric::<Test>(codec, framer)
        .with_reader(r)
        .with_writer(w);

    let origin = Test {
        foo: "hello, world!".to_owned(),
        bar: 114514111,
    };
    framed.send(origin.clone()).await.unwrap();
    framed.send(origin.clone()).await.unwrap();

    let b = ins.lock().await;
    let l = String::from_utf8_lossy(b.as_slice());
    println!("{}", l);
    drop(b);

    let des = framed.next().await.unwrap().unwrap();
    println!("{des:?}");

    assert_eq!(origin, des);
    let des = framed.next().await.unwrap().unwrap();
    println!("{des:?}");

    assert_eq!(origin, des);

    let res = framed.next().await;
    assert!(res.is_none());
}
