use compio::{signal::ctrl_c, time::interval};
use futures_util::{select, FutureExt};
use std::time::Duration;

fn main() {
    compio::task::block_on(async {
        let mut interval = interval(Duration::from_secs(2));
        let mut ctrlc = ctrl_c();
        loop {
            let ctrlc = std::pin::pin!(&mut ctrlc);
            select! {
                res = ctrlc.fuse() => {
                    res.unwrap();
                    println!("break");
                    break;
                },
                _ = interval.tick().fuse() => println!("ping"),
            }
        }
        println!("exit first loop");
        loop {
            interval.tick().await;
            println!("ping");
        }
    })
}
