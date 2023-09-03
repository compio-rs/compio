use async_ctrlc::CtrlC;
use compio::time::interval;
use futures_util::{select, FutureExt};
use std::time::Duration;

fn main() {
    compio::task::block_on(async {
        let mut interval = interval(Duration::from_secs(1));
        let mut ctrlc = CtrlC::new().unwrap();
        loop {
            let ctrlc = std::pin::pin!(&mut ctrlc);
            select! {
                _ = ctrlc.fuse() => break,
                _ = interval.tick().fuse() => println!("ping"),
            }
        }
        println!("exit");
    })
}
