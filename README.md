# prefix-arena

`prefix-arena` is a small `no_std` crate for carving initialized prefixes out of
caller-provided byte storage.

It is aimed at code that needs predictable, allocation-free memory handling:
embedded firmware, packet encoders/decoders, serializers, and other situations
where a growing prefix is useful but a heap allocator is not.

## What it provides

- `PrefixArena` for detaching prefixes from the front of a mutable byte buffer.
- `ArenaView` for temporarily inspecting or initializing the remaining arena.
- `StagingBuffer` for writing into arena-backed storage before committing the
  written prefix.
- `StagingBufferError` for capacity overflow when staging output.

## Features

- `no_std`
- No global allocator
- Backed by caller-owned storage
- Explicit handling of initialized vs uninitialized bytes
- Suitable for incremental writing into fixed-size buffers

## Installation

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
prefix-arena = "0.1"
```

## Basic usage

Detach a prefix directly from the arena:

```rust
use prefix_arena::PrefixArena;

let mut storage = [0u8; 8];
let arena = PrefixArena::new(&mut storage);

let prefix = arena
    .init_prefix_with(|buffer| {
        buffer[..4].copy_from_slice(b"rust");
        Ok::<usize, core::convert::Infallible>(4)
    })
    .unwrap();

assert_eq!(prefix, b"rust");
```

Stage bytes and commit only what was written:

```rust
use prefix_arena::{PrefixArena, StagingBuffer};

let mut storage = [0u8; 16];
let mut arena = PrefixArena::new(&mut storage);
let mut staging = StagingBuffer::new(&mut arena);

staging.extend_from_slice(b"prefix").unwrap();
staging.push_byte(b'-').unwrap();
staging.extend_from_slice(b"arena").unwrap();

let written = staging.into_written_slice();
assert_eq!(written, b"prefix-arena");
assert_eq!(arena.len(), 4);
```

## When to use which type

- Use `PrefixArena` when you already know how many bytes you want to detach.
- Use `ArenaView` when you need temporary access to the remaining buffer.
- Use `StagingBuffer` when output is assembled incrementally and should only be
  committed after the final size is known.

## Safety

This crate intentionally distinguishes between initialized and uninitialized
storage.

Safe methods only expose initialized `u8` slices when the written prefix length
is known. Unsafe methods such as unchecked slice access require the caller to
ensure that the referenced bytes are initialized before any read occurs.

## `no_std`

The crate uses `core` only and is suitable for `no_std` environments.

## Documentation

API documentation is intended to be published on docs.rs:

- <https://docs.rs/prefix-arena>

## License

Licensed under either of:

- MIT license
- Apache License, Version 2.0

at your option.
