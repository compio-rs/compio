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
