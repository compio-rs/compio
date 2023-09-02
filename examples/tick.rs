use compio::time::interval;
use std::time::Duration;

fn main() {
    compio::task::block_on(async {
        let mut interval = interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            println!("ping");
        }
    })
}
