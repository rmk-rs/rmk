use rmk_macro::processor;

#[processor(poll_interval = 0)]
struct PollOnly;

#[processor(subscribe = [], poll_interval = 0, deadline)]
struct PollAndDeadline;

#[processor(subscribe = [])]
#[processor(poll_interval = 0)]
struct SiblingPolling;

fn main() {}
