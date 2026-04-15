#![no_std]
#![doc = include_str!("../README.md")]

mod prefix_arena;
mod staging_buffer;

pub use prefix_arena::{ArenaView, PrefixArena};
pub use staging_buffer::{StagingBuffer, StagingBufferError};

#[cfg(test)]
extern crate std;
