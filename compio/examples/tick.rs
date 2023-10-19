use std::time::Duration;

use compio::{signal::ctrl_c, time::interval};
use futures_util::{select, FutureExt};

#[compio::main(crate = "compio")]
async fn main() {
    let mut interval = interval(Duration::from_secs(2));
    loop {
        let mut ctrlc = ctrl_c();
        let ctrlc = std::pin::pin!(ctrlc);
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
}
