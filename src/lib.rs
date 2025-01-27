//! # Examples
//! ```
//! # use macluhan::Signals;
//! # fn lol() {
//! let mut sigs = Signals::deadly();
//! println!("Got deadly signal {}", sigs.next().unwrap());
//! # }
//! ```
#![cfg_attr(not(feature = "tokio"), no_std)]

#[cfg_attr(target_os = "linux", path = "linux.rs")]
mod os;

pub use os::{Signal, Signals};

#[cfg(feature = "tokio")]
pub use os::tokio;
