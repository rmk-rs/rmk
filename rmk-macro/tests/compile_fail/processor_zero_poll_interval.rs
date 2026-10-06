extern crate self as embassy_time;

pub use rmk::embassy_time::Duration;
use rmk_macro::processor;

#[processor(poll_interval = 0)]
struct PollOnly;

impl PollOnly {
    async fn poll(&mut self) {}
}

fn main() {}
