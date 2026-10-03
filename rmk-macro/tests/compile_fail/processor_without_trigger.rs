use rmk_macro::processor;

#[processor]
struct Bare;

#[processor()]
struct Empty;

#[processor(subscribe = [])]
struct EmptySubscription;

#[processor]
struct MissingMarker;

impl rmk::core_traits::Runnable for MissingMarker {
    async fn run(&mut self) -> ! {
        core::future::pending().await
    }
}

fn main() {}
