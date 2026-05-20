use crate::{ArenaView, PrefixArena};
use core::fmt;
use core::mem::MaybeUninit;

#[inline(always)]
const unsafe fn bytes_as_uninit(slice: &[u8]) -> &[MaybeUninit<u8>] {
    unsafe { core::mem::transmute::<&[u8], &[MaybeUninit<u8>]>(slice) }
}

#[inline(always)]
const unsafe fn uninit_as_bytes(slice: &[MaybeUninit<u8>]) -> &[u8] {
    unsafe { core::mem::transmute::<&[MaybeUninit<u8>], &[u8]>(slice) }
}

#[inline(always)]
const unsafe fn uninit_as_bytes_mut(slice: &mut [MaybeUninit<u8>]) -> &mut [u8] {
    unsafe { core::mem::transmute::<&mut [MaybeUninit<u8>], &mut [u8]>(slice) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingBufferError;

impl fmt::Display for StagingBufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("staging buffer capacity exceeded")
    }
}

/// A staging buffer over the remaining bytes of a [`PrefixArena`].
///
/// `StagingBuffer` lets callers append bytes into temporary arena-backed
/// storage, inspect the written prefix, and finally detach that written prefix
/// from the underlying arena.
pub struct StagingBuffer<'arena, 'b> {
    inner: ArenaView<'arena, 'b>,
    used: usize,
}

impl<'arena, 'b> StagingBuffer<'arena, 'b>
where
    'b: 'arena,
{
    /// Creates a staging buffer over the arena's current remaining space.
    ///
    /// ### Arguments:
    /// - `arena` - The arena whose current remaining bytes will back the staging buffer.
    ///
    /// Note: creating the buffer does not detach any bytes from the arena. Bytes are detached only by [`Self::into_written_slice`].
    ///
    /// ### Returns:
    /// A staging buffer with zero written bytes and capacity equal to the arena's current remaining length.
    ///
    /// ### Panic:
    /// Does not panic.
    pub const fn new(arena: &'arena mut PrefixArena<'b>) -> Self {
        Self {
            inner: arena.view(),
            used: 0,
        }
    }

    /// Appends one byte to the written prefix.
    ///
    /// ### Arguments:
    /// - `byte` - The byte to append.
    ///
    /// ### Returns:
    /// `Ok(())` when one byte is appended.
    /// Returns `Err(StagingBufferError)` when no spare capacity remains, and leaves the written prefix unchanged.
    ///
    /// ### Panic:
    /// Does not panic.
    pub fn push_byte(&mut self, byte: u8) -> Result<(), StagingBufferError> {
        if self.used >= self.inner.len() {
            return Err(StagingBufferError);
        }
        self.inner.as_slice_mut()[self.used].write(byte);
        self.used += 1;
        Ok(())
    }

    /// Appends an entire byte slice to the written prefix.
    ///
    /// ### Arguments:
    /// - `slice` - The bytes to append.
    ///
    /// ### Returns:
    /// `Ok(())` when the full slice fits and is appended.
    /// Returns `Err(StagingBufferError)` when the full slice does not fit, and leaves the written prefix unchanged.
    pub fn extend_from_slice(&mut self, slice: &[u8]) -> Result<(), StagingBufferError> {
        if self.used + slice.len() > self.inner.len() {
            return Err(StagingBufferError);
        }
        self.inner.as_slice_mut()[self.used..self.used + slice.len()]
            .copy_from_slice(unsafe { bytes_as_uninit(slice) });
        self.used += slice.len();
        Ok(())
    }

    /// Appends as much of a slice as fits.
    ///
    /// ### Arguments:
    /// - `slice` - The bytes to append.
    ///
    /// ### Returns:
    /// The number of bytes appended, which may be anywhere from `0` to `slice.len()`.
    pub fn extend_from_slice_capped(&mut self, slice: &[u8]) -> usize {
        let to_fill = core::cmp::min(self.spare_capacity(), slice.len());
        self.inner.as_slice_mut()[self.used..self.used + to_fill]
            .copy_from_slice(unsafe { bytes_as_uninit(&slice[..to_fill]) });
        self.used += to_fill;
        to_fill
    }

    /// Returns the written prefix as an immutable slice.
    ///
    /// ### Returns:
    /// A slice containing exactly the bytes written so far.
    pub fn written(&self) -> &[u8] {
        unsafe { uninit_as_bytes(&self.inner.as_slice()[..self.used]) }
    }

    /// Returns the written prefix as a mutable slice.
    ///
    /// ### Returns:
    /// A mutable slice containing exactly the bytes written so far.
    pub fn written_mut(&mut self) -> &mut [u8] {
        unsafe { uninit_as_bytes_mut(&mut self.inner.as_slice_mut()[..self.used]) }
    }

    /// Returns how many bytes have been written into the buffer.
    ///
    /// ### Returns:
    /// The current length of the written prefix.
    pub const fn len(&self) -> usize {
        self.used
    }

    /// Reports whether no bytes have been written.
    ///
    /// ### Returns:
    /// `true` when `self.len() == 0`, otherwise `false`.
    pub const fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Returns the total capacity of the buffer.
    ///
    /// ### Returns:
    /// The maximum number of bytes that can be written before the buffer reports overflow.
    pub const fn capacity(&self) -> usize {
        self.inner.len()
    }

    /// Returns how many more bytes can be appended without overflowing.
    ///
    /// ### Returns:
    /// `self.capacity() - self.len()`.
    pub const fn spare_capacity(&self) -> usize {
        self.inner.len() - self.used
    }

    /// Marks the buffer as empty without touching the underlying bytes.
    ///
    /// ### Returns:
    /// This method returns `()` and resets `self.len()` to `0`.
    pub fn clear(&mut self) {
        self.used = 0;
    }

    /// Detaches the written prefix from the underlying arena.
    ///
    /// ### Returns:
    /// A mutable slice containing exactly the bytes written so far.
    /// Returns an empty slice when nothing was written.
    pub fn into_written_slice(self) -> &'b mut [u8] {
        unsafe { uninit_as_bytes_mut(self.inner.take_prefix(self.used)) }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::PrefixArena as HeadArena;

    #[test]
    fn test_basic_push() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        assert_eq!(borrowed_buffer.len(), 0);
        assert_eq!(borrowed_buffer.capacity(), 5);
        assert_eq!(borrowed_buffer.spare_capacity(), 5);

        borrowed_buffer.push_byte(42).unwrap();
        assert_eq!(borrowed_buffer.len(), 1);
        assert_eq!(borrowed_buffer.written(), &[42]);
        assert_eq!(borrowed_buffer.spare_capacity(), 4);
    }

    #[test]
    fn test_push_until_full() {
        let mut buffer = [0u8; 3];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.push_byte(1).unwrap();
        borrowed_buffer.push_byte(2).unwrap();
        borrowed_buffer.push_byte(3).unwrap();

        assert_eq!(borrowed_buffer.written(), &[1, 2, 3]);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);
        assert!(borrowed_buffer.push_byte(4).is_err());
    }

    #[test]
    fn test_clear() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[1, 2, 3]).unwrap();
        assert_eq!(borrowed_buffer.len(), 3);

        borrowed_buffer.clear();
        assert_eq!(borrowed_buffer.len(), 0);
        assert_eq!(borrowed_buffer.spare_capacity(), 5);
        assert_eq!(borrowed_buffer.written(), &[]);
    }

    #[test]
    fn test_as_mut_slice() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[1, 2, 3]).unwrap();

        let mut_slice = borrowed_buffer.written_mut();
        mut_slice[1] = 99;

        assert_eq!(borrowed_buffer.written(), &[1, 99, 3]);
    }

    #[test]
    fn test_empty_buffer() {
        let mut buffer = [0u8; 0];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        assert_eq!(borrowed_buffer.len(), 0);
        assert_eq!(borrowed_buffer.capacity(), 0);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);
        assert!(borrowed_buffer.push_byte(1).is_err());
        assert!(borrowed_buffer.extend_from_slice(&[1]).is_err());
        assert_eq!(borrowed_buffer.written(), &[]);
    }

    #[test]
    fn test_single_byte_buffer() {
        let mut buffer = [0u8; 1];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        assert_eq!(borrowed_buffer.capacity(), 1);
        assert_eq!(borrowed_buffer.spare_capacity(), 1);

        borrowed_buffer.push_byte(42).unwrap();
        assert_eq!(borrowed_buffer.written(), &[42]);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);

        assert!(borrowed_buffer.push_byte(1).is_err());
        assert!(borrowed_buffer.extend_from_slice(&[1]).is_err());
    }

    #[test]
    fn test_clear_and_reuse() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[1, 2, 3]).unwrap();
        assert_eq!(borrowed_buffer.len(), 3);

        borrowed_buffer.clear();
        borrowed_buffer.extend_from_slice(&[4, 5, 6, 7, 8]).unwrap();
        assert_eq!(borrowed_buffer.written(), &[4, 5, 6, 7, 8]);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);
    }

    #[test]
    fn test_extend_at_exact_capacity() {
        let mut buffer = [0u8; 3];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        assert!(borrowed_buffer.extend_from_slice(&[1, 2, 3]).is_ok());
        assert_eq!(borrowed_buffer.len(), 3);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);

        assert!(borrowed_buffer.extend_from_slice(&[]).is_ok());
        assert_eq!(borrowed_buffer.len(), 3);
    }

    #[test]
    fn test_take_used() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[10, 20, 30]).unwrap();
        let used = borrowed_buffer.into_written_slice();

        assert_eq!(used, &[10, 20, 30]);
        used[0] = 255;
        assert_eq!(used, &[255, 20, 30]);
    }

    #[test]
    fn test_extend_empty_slice() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[]).unwrap();
        assert_eq!(borrowed_buffer.len(), 0);
        assert_eq!(borrowed_buffer.written(), &[]);
    }

    #[test]
    fn test_capacity_overflow() {
        let mut buffer = [0u8; 2];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        assert!(borrowed_buffer.extend_from_slice(&[1, 2, 3]).is_err());
        assert_eq!(borrowed_buffer.len(), 0);

        borrowed_buffer.extend_from_slice(&[1, 2]).unwrap();
        assert!(borrowed_buffer.push_byte(3).is_err());
        assert_eq!(borrowed_buffer.written(), &[1, 2]);
    }

    #[test]
    fn test_extend_error_preserves_written_prefix() {
        let mut buffer = [0u8; 4];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[1, 2]).unwrap();
        assert!(borrowed_buffer.extend_from_slice(&[3, 4, 5]).is_err());

        assert_eq!(borrowed_buffer.written(), &[1, 2]);
        assert_eq!(borrowed_buffer.len(), 2);
        assert_eq!(borrowed_buffer.spare_capacity(), 2);
    }

    #[test]
    fn test_multiple_extension() {
        let mut buffer = [0u8; 10];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[1, 2, 3]).unwrap();
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3]);

        borrowed_buffer.extend_from_slice(&[4, 5]).unwrap();
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3, 4, 5]);

        borrowed_buffer
            .extend_from_slice(&[6, 7, 8, 9, 10])
            .unwrap();
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        assert!(borrowed_buffer.extend_from_slice(&[11]).is_err());
    }

    #[test]
    fn test_append_from_slice() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        // Append with data larger than remaining capacity
        let filled = borrowed_buffer.extend_from_slice_capped(&[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(filled, 5);
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3, 4, 5]);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);

        // Try to append when buffer is full
        let filled = borrowed_buffer.extend_from_slice_capped(&[8, 9]);
        assert_eq!(filled, 0);
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_append_from_slice_partial() {
        let mut buffer = [0u8; 10];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        // Add some data first
        borrowed_buffer.extend_from_slice(&[1, 2, 3]).unwrap();
        assert_eq!(borrowed_buffer.spare_capacity(), 7);

        // Append with smaller slice
        let filled = borrowed_buffer.extend_from_slice_capped(&[4, 5]);
        assert_eq!(filled, 2);
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3, 4, 5]);
        assert_eq!(borrowed_buffer.spare_capacity(), 5);

        // Append remaining with exact size
        let filled = borrowed_buffer.extend_from_slice_capped(&[6, 7, 8, 9, 10]);
        assert_eq!(filled, 5);
        assert_eq!(borrowed_buffer.written(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(borrowed_buffer.spare_capacity(), 0);
    }

    #[test]
    fn test_fill_remaining_empty_slice() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        let filled = borrowed_buffer.extend_from_slice_capped(&[]);
        assert_eq!(filled, 0);
        assert_eq!(borrowed_buffer.len(), 0);
    }

    #[test]
    fn test_as_slice_lifetime() {
        let mut buffer = [1, 2, 3, 4, 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);
        borrowed_buffer.extend_from_slice(&[1, 2, 3]).unwrap();

        let slice1 = borrowed_buffer.written();
        let slice2 = borrowed_buffer.written();

        assert_eq!(slice1, slice2);
        assert_eq!(slice1, &[1, 2, 3]);
    }

    #[test]
    fn test_into_written_slice_advances_arena() {
        let mut buffer = [0u8; 5];
        let mut allocator = HeadArena::new(&mut buffer);
        let mut borrowed_buffer = StagingBuffer::new(&mut allocator);

        borrowed_buffer.extend_from_slice(&[10, 20, 30]).unwrap();
        let written = borrowed_buffer.into_written_slice();

        assert_eq!(written, &[10, 20, 30]);
        assert_eq!(allocator.len(), 2);
        let remaining = unsafe { uninit_as_bytes(allocator.take_remaining()) };
        assert_eq!(remaining, &[0, 0]);
    }

    #[test]
    fn test_into_written_slice_empty_keeps_arena_unchanged() {
        let mut buffer = [0u8; 3];
        let mut allocator = HeadArena::new(&mut buffer);
        let borrowed_buffer = StagingBuffer::new(&mut allocator);

        let written = borrowed_buffer.into_written_slice();

        assert_eq!(written, &[]);
        assert_eq!(allocator.len(), 3);
    }
}
