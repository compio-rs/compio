use compio as compio_alias;
use compio_driver::DriverType;

#[compio::test]
async fn simple_main() {}

#[compio::test(crate = compio)]
async fn simple_main_with_crate() {}

#[compio::test(crate = "compio")]
async fn simple_main_with_crate_str() {}

#[compio::test(crate = compio_alias)]
async fn simple_main_with_alias() {}

#[compio::test(event_interval = 8)]
async fn main_with_runtime_args() {}

#[compio::test(crate = compio_alias, event_interval = 8, with_proactor(driver_type = DriverType::Poll))]
async fn main_with_multiple_args() {}

/// The `console` argument, which is compiled but never run: it installs a
/// process-wide subscriber, which is why `#[compio::test]` rejects it, and
/// running one here would install it in a binary full of tests.
#[compio::main(console, event_interval = 8)]
async fn main_with_console() {}
