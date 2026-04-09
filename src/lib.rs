#![no_std]
//! Prefix-oriented arena utilities for caller-provided byte buffers.
//!
//! `prefix-arena` is a small `no_std` crate for working with a mutable byte
//! buffer as a prefix-growing arena. It is designed for cases where allocation
//! must stay explicit and bounded, such as embedded systems, protocol parsing,
//! serializers, and staging temporary output into preallocated storage.
//!
//! The crate exposes two primary types:
//!
//! - [`PrefixArena`]: detaches prefixes from a backing buffer while keeping the
//!   remaining capacity available for later use.
//! - [`StagingBuffer`]: writes into the current arena prefix and detaches only
//!   the bytes that were actually produced.
//!
//! # Examples
//!
//! Reserve a prefix directly from the arena:
//!
//! ```
//! use prefix_arena::PrefixArena;
//!
//! let mut storage = [0u8; 8];
//! let arena = PrefixArena::new(&mut storage);
//!
//! let prefix = arena
//!     .init_prefix_with(|buffer| {
//!         buffer[..3].copy_from_slice(b"abc");
//!         Ok::<usize, core::convert::Infallible>(3)
//!     })
//!     .unwrap();
//!
//! assert_eq!(prefix, b"abc");
//! ```
//!
//! Stage bytes first and detach them once complete:
//!
//! ```
//! use prefix_arena::{PrefixArena, StagingBuffer};
//!
//! let mut storage = [0u8; 16];
//! let mut arena = PrefixArena::new(&mut storage);
//! let mut staging = StagingBuffer::new(&mut arena);
//!
//! staging.extend_from_slice(b"hello").unwrap();
//! staging.push_byte(b'!').unwrap();
//!
//! let written = staging.into_written_slice();
//! assert_eq!(written, b"hello!");
//! assert_eq!(arena.len(), 10);
//! ```
//!
//! # Safety model
//!
//! The safe API only returns initialized `&[u8]` or `&mut [u8]` values after the
//! caller reports how many bytes were written. Unsafe methods exist for treating
//! the remaining arena bytes as initialized data; those methods require the
//! caller to uphold initialization guarantees.
mod prefix_arena;
mod staging_buffer;

pub use prefix_arena::{ArenaView, PrefixArena};
pub use staging_buffer::{StagingBuffer, StagingBufferError};

#[cfg(test)]
extern crate std;
