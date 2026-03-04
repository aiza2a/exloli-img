pub mod command;
pub mod utils;
mod dispatcher;
mod filter;
mod handlers;
mod scheduler;

pub use dispatcher::start_dispatcher;
use teloxide::adaptors::{CacheMe, DefaultParseMode, Throttle};

pub type Bot = CacheMe<DefaultParseMode<Throttle<teloxide::Bot>>>;
