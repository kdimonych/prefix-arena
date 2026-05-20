use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;

#[inline(always)]
const unsafe fn bytes_as_uninit_mut(slice: &mut [u8]) -> &mut [MaybeUninit<u8>] {
    unsafe { core::mem::transmute::<&mut [u8], &mut [MaybeUninit<u8>]>(slice) }
}

#[inline(always)]
const unsafe fn assume_init_as_bytes(slice: &[MaybeUninit<u8>]) -> &[u8] {
    unsafe { core::mem::transmute::<&[MaybeUninit<u8>], &[u8]>(slice) }
}

#[inline(always)]
const unsafe fn assume_init_as_bytes_mut(slice: &mut [MaybeUninit<u8>]) -> &mut [u8] {
    unsafe { core::mem::transmute::<&mut [MaybeUninit<u8>], &mut [u8]>(slice) }
}

/// A bump-style arena over caller-provided byte storage.
///
/// `PrefixArena` hands out slices from the front of the remaining buffer and keeps
/// the rest available for later use. It only manages buffer boundaries; it does
/// not track which bytes are initialized.
pub struct PrefixArena<'buf> {
    remaining: UnsafeCell<&'buf mut [MaybeUninit<u8>]>,
}

impl<'buf> PrefixArena<'buf> {
    /// Creates a prefix arena over initialized byte storage.
    ///
    /// ### Arguments:
    /// - `arena` - The backing byte slice that the arena will hand out from the front.
    ///
    /// ### Returns:
    /// A new arena whose remaining capacity is `arena.len()`.
    #[inline(always)]
    #[must_use]
    pub const fn new(arena: &'buf mut [u8]) -> Self {
        Self::from_uninit(unsafe { bytes_as_uninit_mut(arena) })
    }

    /// Creates a prefix arena over possibly uninitialized byte storage.
    ///
    /// ### Arguments:
    /// - `arena` - The backing storage that the arena will hand out from the front.
    ///
    /// ### Returns:
    /// A new arena whose remaining capacity is `arena.len()`.
    #[inline]
    #[must_use]
    pub const fn from_uninit(arena: &'buf mut [MaybeUninit<u8>]) -> Self {
        Self {
            remaining: UnsafeCell::new(arena),
        }
    }

    /// Returns how many bytes are still available in the arena.
    ///
    /// ### Returns:
    /// The length of the remaining, not-yet-detached prefix.
    #[inline]
    pub const fn len(&self) -> usize {
        let remaining = unsafe { &*self.remaining.get() };
        remaining.len()
    }

    /// Reports whether the arena has no remaining bytes.
    ///
    /// ### Returns:
    /// `true` when `self.len() == 0`, otherwise `false`.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Creates a temporary view over the arena's remaining bytes.
    ///
    /// ### Returns:
    /// An [`ArenaView`] pointing at the same remaining bytes tracked by this arena.
    #[must_use]
    pub const fn view<'arena>(&'arena mut self) -> ArenaView<'arena, 'buf> {
        // SAFETY: `self.remaining` points at the remaining part of the arena for `'buf`.
        ArenaView::<'arena, 'buf>::new(self.remaining.get())
    }

    /// Detaches the first `n` bytes from the arena.
    ///
    /// ### Arguments:
    /// - `n` - The number of bytes to remove from the front of the remaining arena.
    ///
    /// Note: the returned slice is uninitialized storage. Initialize it before reading it as `u8`.
    ///
    /// ### Returns:
    /// A mutable slice covering exactly the detached prefix.
    ///
    /// ### Panic:
    /// Panics if `n > self.len()`.
    pub fn take_prefix(&self, n: usize) -> &'buf mut [MaybeUninit<u8>] {
        let buffer = unsafe { &mut *self.remaining.get() };
        let (used, remaining) = buffer.split_at_mut(n);
        unsafe { *self.remaining.get() = remaining };
        used
    }

    /// Tries to detach the first `n` bytes from the arena.
    ///
    /// ### Arguments:
    /// - `n` - The number of bytes to remove from the front of the remaining arena.
    ///
    /// Note: the returned slice is uninitialized storage. Initialize it before reading it as `u8`.
    ///
    /// ### Returns:
    /// `Some(...)` with a mutable slice of exactly `n` bytes when enough capacity remains.
    /// Returns `None` when `n > self.len()`, and leaves the arena unchanged.
    pub fn take_prefix_checked(&self, n: usize) -> Option<&'buf mut [MaybeUninit<u8>]> {
        let buffer = unsafe { &mut *self.remaining.get() };
        let (used, remaining) = buffer.split_at_mut_checked(n)?;
        unsafe { *self.remaining.get() = remaining };
        Some(used)
    }

    /// Detaches the first `n` bytes from the arena as `u8` without checks.
    ///
    /// ### Arguments:
    /// - `n` - The number of bytes to remove from the front of the remaining arena.
    ///
    /// ### Returns:
    /// A mutable `u8` slice covering exactly the detached prefix.
    ///
    /// ### Safety
    ///
    /// `n` must be less than or equal to `self.len()`, and every returned byte must be initialized before it is read as `u8`.
    #[inline(always)]
    pub unsafe fn take_prefix_unchecked(&self, n: usize) -> &'buf mut [u8] {
        let buffer = unsafe { &mut *self.remaining.get() };
        let (used, remaining) = unsafe { buffer.split_at_mut_unchecked(n) };
        unsafe { *self.remaining.get() = remaining };
        unsafe { assume_init_as_bytes_mut(used) }
    }

    /// Returns all bytes that still remain in the arena and consumes it.
    ///
    /// Note: the returned slice is uninitialized storage. Initialize it before reading it as `u8`.
    ///
    /// ### Returns:
    /// The full remaining portion of the backing storage.
    #[must_use]
    pub fn take_remaining(self) -> &'buf mut [MaybeUninit<u8>] {
        self.remaining.into_inner()
    }

    /// Initializes and detaches a prefix chosen by a callback.
    ///
    /// ### Arguments:
    /// - `f` - A callback that receives the full remaining storage and returns the length of the initialized prefix.
    ///
    /// Note: when `f` returns `Err`, no prefix is detached from the backing storage, although bytes written by `f` stay in that storage.
    ///
    /// ### Returns:
    /// `Ok(...)` with the detached initialized prefix when `f` succeeds with a valid length.
    /// Returns `Err(E)` when `f` returns `Err(E)`, and detaches no bytes from the backing storage.
    ///
    /// ### Panic:
    /// Panics if `f` panics or if it returns a length greater than the current remaining capacity.
    ///
    /// ### Safety:
    /// `f` must report only a prefix that it fully initialized. Reporting uninitialized bytes as initialized makes later reads through the returned `&mut [u8]` invalid.
    pub fn init_prefix_with<F, E>(self, f: F) -> Result<&'buf mut [u8], E>
    where
        F: FnOnce(&mut [MaybeUninit<u8>]) -> Result<usize, E>,
    {
        let buffer: &mut [MaybeUninit<u8>] =
            unsafe { self.remaining.get().as_mut().unwrap_unchecked() };
        let initialized_len = f(buffer)?;
        if initialized_len > buffer.len() {
            panic!("Initializer function returned a length greater than the current buffer size");
        }

        Ok(unsafe { self.take_prefix_unchecked(initialized_len) })
    }

    /// Initializes and detaches a prefix chosen by a callback, without panicking on oversized lengths.
    ///
    /// ### Arguments:
    /// - `f` - A callback that receives the full remaining storage and returns the length of the initialized prefix.
    ///
    /// Note: when `f` returns `Err`, no prefix is detached from the backing storage, although bytes written by `f` stay in that storage.
    ///
    /// ### Returns:
    /// `Ok(Some(...))` with the detached initialized prefix when `f` succeeds with a valid length.
    /// Returns `Ok(None)` when `f` returns a length greater than the current remaining capacity, and detaches no bytes from the backing storage.
    /// Returns `Err(E)` when `f` returns `Err(E)`, and detaches no bytes from the backing storage.
    ///
    /// ### Panic:
    /// Panics only if `f` panics.
    ///
    /// ### Safety:
    /// `f` must report only a prefix that it fully initialized. Reporting uninitialized bytes as initialized makes later reads through the returned `&mut [u8]` invalid.
    pub fn init_prefix_with_checked<F, E>(self, f: F) -> Result<Option<&'buf mut [u8]>, E>
    where
        F: FnOnce(&mut [MaybeUninit<u8>]) -> Result<usize, E>,
    {
        let buffer: &mut [MaybeUninit<u8>] =
            unsafe { self.remaining.get().as_mut().unwrap_unchecked() };
        let initialized_len = f(buffer)?;
        if initialized_len > buffer.len() {
            return Ok(None);
        }

        Ok(Some(unsafe { self.take_prefix_unchecked(initialized_len) }))
    }
}

/// A temporary view over the remaining bytes of a [`PrefixArena`].
///
/// This is useful when code needs to fill an unknown-sized prefix of the
/// remaining arena and then detach only the initialized portion.
pub struct ArenaView<'arena, 'buf>
where
    'buf: 'arena,
{
    remaining: *mut &'buf mut [MaybeUninit<u8>],
    _marker: PhantomData<&'arena ()>,
}

impl<'arena, 'buf> ArenaView<'arena, 'buf> {
    /// Creates a temporary view over the remaining arena bytes.
    ///
    /// ### Returns:
    /// A view pointing at the same remaining bytes tracked by the originating arena.
    #[must_use]
    const fn new(remaining: *mut &'buf mut [MaybeUninit<u8>]) -> Self {
        Self {
            remaining,
            _marker: PhantomData,
        }
    }

    /// Returns how many bytes are currently visible through this view.
    ///
    /// ### Returns:
    /// The length of the remaining bytes shared with the underlying arena.
    #[inline]
    pub const fn len(&self) -> usize {
        let remaining = unsafe { self.remaining.as_mut().unwrap_unchecked() };
        remaining.len()
    }

    /// Reports whether this view is empty.
    ///
    /// ### Returns:
    /// `true` when `self.len() == 0`, otherwise `false`.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the remaining bytes as immutable uninitialized storage.
    ///
    /// ### Returns:
    /// An immutable slice over the bytes currently visible through this view.
    #[inline]
    pub const fn as_uninit_slice(&self) -> &[MaybeUninit<u8>] {
        self.as_slice()
    }

    /// Returns the remaining bytes as immutable uninitialized storage.
    ///
    /// ### Returns:
    /// An immutable slice over the bytes currently visible through this view.
    #[inline]
    pub const fn as_slice(&self) -> &[MaybeUninit<u8>] {
        unsafe { self.remaining.as_mut().unwrap_unchecked() }
    }

    /// Returns the remaining bytes as immutable `u8` without checking initialization.
    ///
    /// ### Returns:
    /// An immutable `u8` slice over the bytes currently visible through this view.
    ///
    /// ### Safety
    ///
    /// Every returned byte must already be initialized before it is read as `u8`.
    #[inline]
    pub const unsafe fn as_slice_unchecked(&self) -> &[u8] {
        unsafe { assume_init_as_bytes(self.as_slice()) }
    }

    /// Returns the remaining bytes as mutable uninitialized storage.
    ///
    /// ### Returns:
    /// A mutable slice over the bytes currently visible through this view.
    #[inline]
    pub const fn as_uninit_slice_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        self.as_slice_mut()
    }

    /// Returns the remaining bytes as mutable uninitialized storage.
    ///
    /// ### Returns:
    /// A mutable slice over the bytes currently visible through this view.
    #[inline]
    pub const fn as_slice_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        unsafe { self.remaining.as_mut().unwrap_unchecked() }
    }

    /// Returns the remaining bytes as mutable `u8` without checking initialization.
    ///
    /// ### Returns:
    /// A mutable `u8` slice over the bytes currently visible through this view.
    ///
    /// ### Safety
    ///
    /// Every returned byte must already be initialized before it is read as `u8`.
    #[inline]
    pub const unsafe fn as_slice_mut_unchecked(&mut self) -> &mut [u8] {
        unsafe { assume_init_as_bytes_mut(self.as_slice_mut()) }
    }

    /// Initializes a prefix inside the current view without shrinking the arena.
    ///
    /// ### Arguments:
    /// - `f` - A callback that receives the full visible storage and returns the length of the initialized prefix.
    ///
    /// Note: this method does not detach bytes from the underlying arena.
    ///
    /// ### Returns:
    /// `Ok(...)` with the initialized prefix when `f` succeeds with a valid length.
    /// Returns `Err(E)` when `f` returns `Err(E)`, and leaves the visible length unchanged.
    ///
    /// ### Panic:
    /// Panics if `f` panics or if it returns a length greater than `self.len()`.
    ///
    /// ### Safety:
    /// `f` must report only a prefix that it fully initialized. Reporting uninitialized bytes as initialized makes later reads through the returned `&mut [u8]` invalid.
    pub fn init_with<F, E>(&mut self, f: F) -> Result<&mut [u8], E>
    where
        F: FnOnce(&mut [MaybeUninit<u8>]) -> Result<usize, E>,
    {
        let slice = self.as_slice_mut();
        let initialized_len = f(slice)?;
        if initialized_len > slice.len() {
            panic!("Initializer function returned a length greater than the current buffer size");
        }
        Ok(unsafe { assume_init_as_bytes_mut(&mut slice[..initialized_len]) })
    }

    /// Initializes a prefix inside the current view without shrinking the arena, returning `None` for oversized lengths.
    ///
    /// ### Arguments:
    /// - `f` - A callback that receives the full visible storage and returns the length of the initialized prefix.
    ///
    /// Note: this method does not detach bytes from the underlying arena.
    ///
    /// ### Returns:
    /// `Ok(Some(...))` with the initialized prefix when `f` succeeds with a valid length.
    /// Returns `Ok(None)` when `f` returns a length greater than `self.len()`, and leaves the visible bytes available.
    /// Returns `Err(E)` when `f` returns `Err(E)`, and leaves the visible length unchanged.
    ///
    /// ### Panic:
    /// Panics only if `f` panics.
    ///
    /// ### Safety:
    /// `f` must report only a prefix that it fully initialized. Reporting uninitialized bytes as initialized makes later reads through the returned `&mut [u8]` invalid.
    pub fn init_with_checked<F, E>(&mut self, f: F) -> Result<Option<&mut [u8]>, E>
    where
        F: FnOnce(&mut [MaybeUninit<u8>]) -> Result<usize, E>,
    {
        let slice = self.as_slice_mut();
        let initialized_len = f(slice)?;
        if initialized_len > slice.len() {
            return Ok(None);
        }
        Ok(unsafe { Some(assume_init_as_bytes_mut(&mut slice[..initialized_len])) })
    }

    /// Initializes a prefix inside the current view and detaches it from the arena.
    ///
    /// ### Arguments:
    /// - `f` - A callback that receives the full visible storage and returns the length of the initialized prefix.
    ///
    /// Note: when `f` returns `Err`, the underlying arena length is unchanged, although bytes written by `f` stay in the backing storage.
    ///
    /// ### Returns:
    /// `Ok(...)` with the detached initialized prefix when `f` succeeds with a valid length.
    /// Returns `Err(E)` when `f` returns `Err(E)`, and leaves the arena length unchanged.
    ///
    /// ### Panic:
    /// Panics if `f` panics or if it returns a length greater than `self.len()`.
    ///
    /// ### Safety:
    /// `f` must report only a prefix that it fully initialized. Reporting uninitialized bytes as initialized makes later reads through the returned `&mut [u8]` invalid.
    pub fn init_prefix_with<F, E>(mut self, f: F) -> Result<&'buf mut [u8], E>
    where
        F: FnOnce(&mut [MaybeUninit<u8>]) -> Result<usize, E>,
    {
        let slice = self.as_slice_mut();
        let initialized_len = f(slice)?;

        if initialized_len > slice.len() {
            panic!("Initializer function returned a length greater than the current buffer size");
        }

        Ok(unsafe { self.take_prefix_unchecked(initialized_len) })
    }

    /// Initializes a prefix inside the current view and detaches it from the arena, returning `None` for oversized lengths.
    ///
    /// ### Arguments:
    /// - `f` - A callback that receives the full visible storage and returns the length of the initialized prefix.
    ///
    /// Note: when `f` returns `Err`, the underlying arena length is unchanged, although bytes written by `f` stay in the backing storage.
    ///
    /// ### Returns:
    /// `Ok(Some(...))` with the detached initialized prefix when `f` succeeds with a valid length.
    /// Returns `Ok(None)` when `f` returns a length greater than `self.len()`, and leaves the arena unchanged.
    /// Returns `Err(E)` when `f` returns `Err(E)`, and leaves the arena length unchanged.
    ///
    /// ### Panic:
    /// Panics only if `f` panics.
    ///
    /// ### Safety:
    /// `f` must report only a prefix that it fully initialized. Reporting uninitialized bytes as initialized makes later reads through the returned `&mut [u8]` invalid.
    pub fn init_prefix_with_checked<F, E>(mut self, f: F) -> Result<Option<&'buf mut [u8]>, E>
    where
        F: FnOnce(&mut [MaybeUninit<u8>]) -> Result<usize, E>,
    {
        let slice = self.as_slice_mut();
        let initialized_len = f(slice)?;

        if initialized_len > slice.len() {
            return Ok(None);
        }

        Ok(Some(unsafe { self.take_prefix_unchecked(initialized_len) }))
    }

    /// Detaches the first `n` bytes from the underlying arena.
    ///
    /// ### Arguments:
    /// - `n` - The number of bytes to remove from the front of the shared remaining storage.
    ///
    /// Note: the returned slice is uninitialized storage. Initialize it before reading it as `u8`.
    ///
    /// ### Returns:
    /// A mutable slice covering exactly the detached prefix.
    ///
    /// ### Panic:
    /// Panics if `n > self.len()`.
    #[must_use]
    pub fn take_prefix(self, n: usize) -> &'buf mut [MaybeUninit<u8>] {
        let buffer = unsafe { &mut *self.remaining.as_mut().unwrap_unchecked() };
        let (used, remaining) = buffer.split_at_mut(n);
        unsafe { *self.remaining.as_mut().unwrap_unchecked() = remaining };
        used
    }

    /// Detaches the first `n` bytes from the underlying arena as `u8` without checks.
    ///
    /// ### Arguments:
    /// - `n` - The number of bytes to remove from the front of the shared remaining storage.
    ///
    /// ### Returns:
    /// A mutable `u8` slice covering exactly the detached prefix.
    ///
    /// ### Safety
    ///
    /// `n` must be less than or equal to `self.len()`, and every returned byte must be initialized before it is read as `u8`.
    #[inline(always)]
    #[must_use]
    pub unsafe fn take_prefix_unchecked(self, n: usize) -> &'buf mut [u8]
    where
        'buf: 'arena,
    {
        let buffer = unsafe { &mut *self.remaining.as_mut().unwrap_unchecked() };
        let (used, remaining) = unsafe { buffer.split_at_mut_unchecked(n) };
        unsafe { *self.remaining.as_mut().unwrap_unchecked() = remaining };
        unsafe { assume_init_as_bytes_mut(used) }
    }
}

impl<'buf> From<&'buf mut [u8]> for PrefixArena<'buf> {
    fn from(buffer: &'buf mut [u8]) -> Self {
        Self::new(buffer)
    }
}

impl<'buf> From<&'buf mut [MaybeUninit<u8>]> for PrefixArena<'buf> {
    fn from(buffer: &'buf mut [MaybeUninit<u8>]) -> Self {
        Self::from_uninit(buffer)
    }
}

impl<'buf, const N: usize> From<&'buf mut [u8; N]> for PrefixArena<'buf> {
    fn from(buffer: &'buf mut [u8; N]) -> Self {
        Self::new(&mut buffer[..])
    }
}

impl<'buf, const N: usize> From<&'buf mut [MaybeUninit<u8>; N]> for PrefixArena<'buf> {
    fn from(buffer: &'buf mut [MaybeUninit<u8>; N]) -> Self {
        Self::from_uninit(&mut buffer[..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestError {
        Expected,
    }

    #[test]
    fn test_init_from_initialized() {
        let mut data = [1, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::new(&mut data);
        assert_eq!(prefix_arena.len(), 5);
        assert!(!prefix_arena.is_empty());
    }

    #[test]
    fn test_init_from_uninitialized() {
        let mut data = [1, 2, 3, 4, 5].map(MaybeUninit::new);
        let prefix_arena = PrefixArena::from_uninit(&mut data);
        assert_eq!(prefix_arena.len(), 5);
        assert!(!prefix_arena.is_empty());
    }

    #[test]
    fn test_from_slice() {
        let mut data = [1, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::new(&mut data[..]);
        assert_eq!(prefix_arena.len(), 5);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn test_from_uninitialized_slice() {
        let data: [u8; 5] = [1, 2, 3, 4, 5];
        let mut uninit_data = data.map(MaybeUninit::new);

        let prefix_arena = PrefixArena::from_uninit(&mut uninit_data);
        assert_eq!(prefix_arena.len(), 5);
        assert!(!prefix_arena.is_empty());
    }

    #[test]
    fn test_from_array() {
        let mut data = [1, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::from(&mut data);
        assert_eq!(prefix_arena.len(), 5);
    }

    #[test]
    fn test_from_array_for_temporary_buffer() {
        let mut data = [1, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::from(&mut data);
        assert_eq!(prefix_arena.len(), 5);
        assert_eq!(prefix_arena.view().len(), 5);
    }

    #[test]
    fn test_empty_buffer() {
        let mut data = [];
        let prefix_arena = PrefixArena::new(&mut data);
        assert_eq!(prefix_arena.len(), 0);
        assert!(prefix_arena.is_empty());
    }

    #[test]
    fn test_empty_buffer_for_temporary_buffer() {
        let mut data = [];
        let mut prefix_arena = PrefixArena::new(&mut data);
        assert_eq!(prefix_arena.len(), 0);
        assert!(prefix_arena.is_empty());
        let temp_buffer = prefix_arena.view();
        assert_eq!(temp_buffer.len(), 0);
        assert!(temp_buffer.is_empty());
    }

    #[test]
    fn test_temp_buffer_as_slice() {
        let mut data = [1, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let mut temp_buffer = prefix_arena.view();
        assert_eq!(
            unsafe { assume_init_as_bytes(temp_buffer.as_slice_mut()) },
            &[1, 2, 3, 4, 5]
        );
    }
    #[test]
    fn test_temp_buffer_as_slice_unchecked() {
        let mut data = [1, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();
        assert_eq!(
            unsafe { temp_buffer.as_slice_unchecked() },
            &[1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn test_temp_buffer_as_mut_slice() {
        let mut data = [1, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let mut temp_buffer = prefix_arena.view();
        let slice = temp_buffer.as_slice_mut();
        slice[0].write(10);
        assert_eq!(
            unsafe { temp_buffer.as_slice_unchecked() },
            &[10, 2, 3, 4, 5]
        );
    }

    #[test]
    fn test_temp_buffer_as_mut_slice_unchecked() {
        let mut data = [1, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let mut temp_buffer = prefix_arena.view();
        let slice = unsafe { temp_buffer.as_slice_mut_unchecked() };
        slice[0] = 10;
        assert_eq!(
            unsafe { temp_buffer.as_slice_unchecked() },
            &[10, 2, 3, 4, 5]
        );
    }

    #[test]
    fn test_prefix_arena_init_then_acquire_with() {
        let mut data = [0u8; 5];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = prefix_arena
            .init_prefix_with(|buffer| {
                buffer[..3].write_copy_of_slice(&[7, 8, 9]);
                Ok::<usize, TestError>(3)
            })
            .unwrap();

        assert_eq!(detached, &[7, 8, 9]);
    }

    #[test]
    fn test_prefix_arena_init_then_acquire_with_error_preserves_len() {
        let mut data = [1u8, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::new(&mut data);

        let error = prefix_arena
            .init_prefix_with(|buffer| {
                buffer[0].write(99);
                Err::<usize, TestError>(TestError::Expected)
            })
            .unwrap_err();

        assert_eq!(error, TestError::Expected);
        assert_eq!(data, [99, 2, 3, 4, 5]);
    }

    #[test]
    fn test_prefix_arena_init_then_acquire_with_checked() {
        let mut data = [0u8; 5];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = prefix_arena
            .init_prefix_with_checked(|buffer| {
                buffer[..3].write_copy_of_slice(&[7, 8, 9]);
                Ok::<usize, TestError>(3)
            })
            .unwrap()
            .unwrap();

        assert_eq!(detached, &[7, 8, 9]);
    }

    #[test]
    fn test_prefix_arena_init_then_acquire_with_checked_invalid_len_preserves_arena() {
        let mut data = [1u8, 2, 3];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = prefix_arena
            .init_prefix_with_checked(|buffer| {
                buffer[0].write(9);
                Ok::<usize, TestError>(4)
            })
            .unwrap();

        assert_eq!(detached, None);
        assert_eq!(data, [9, 2, 3]);
    }

    #[test]
    #[should_panic(
        expected = "Initializer function returned a length greater than the current buffer size"
    )]
    fn test_prefix_arena_init_then_acquire_with_panics_on_invalid_len() {
        let mut data = [0u8; 3];
        let prefix_arena = PrefixArena::new(&mut data);

        let _ = prefix_arena.init_prefix_with(|_| Ok::<usize, TestError>(4));
    }

    #[test]
    fn test_temp_buffer_as_mut_with_init() {
        let mut data = [0u8; 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let mut temp_buffer = prefix_arena.view();

        let initialized = temp_buffer
            .init_with(|buffer| {
                buffer[..2].write_copy_of_slice(&[11, 12]);
                Ok::<usize, TestError>(2)
            })
            .unwrap();

        assert_eq!(initialized, &[11, 12]);
        assert_eq!(temp_buffer.len(), 5);
        assert_eq!(prefix_arena.len(), 5);
    }

    #[test]
    fn test_temp_buffer_as_mut_with_init_checked() {
        let mut data = [0u8; 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let mut temp_buffer = prefix_arena.view();

        let initialized = temp_buffer
            .init_with_checked(|buffer| {
                buffer[..2].write_copy_of_slice(&[11, 12]);
                Ok::<usize, TestError>(2)
            })
            .unwrap()
            .unwrap();

        assert_eq!(initialized, &[11, 12]);
        assert_eq!(temp_buffer.len(), 5);
        assert_eq!(prefix_arena.len(), 5);
    }

    #[test]
    fn test_temp_buffer_as_mut_with_init_checked_invalid_len() {
        let mut data = [1u8, 2, 3];
        let mut prefix_arena = PrefixArena::new(&mut data);
        {
            let mut temp_buffer = prefix_arena.view();

            let initialized = temp_buffer
                .init_with_checked(|buffer| {
                    buffer[0].write(9);
                    Ok::<usize, TestError>(4)
                })
                .unwrap();

            assert_eq!(initialized, None);
            assert_eq!(unsafe { temp_buffer.as_slice_unchecked() }, &[9, 2, 3]);
        }
        assert_eq!(prefix_arena.len(), 3);
    }

    #[test]
    fn test_temp_buffer_init_then_acquire_with() {
        let mut data = [0u8; 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let detached = temp_buffer
            .init_prefix_with(|buffer| {
                buffer[..2].write_copy_of_slice(&[21, 22]);
                Ok::<usize, TestError>(2)
            })
            .unwrap();

        assert_eq!(detached, &[21, 22]);
        assert_eq!(prefix_arena.len(), 3);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[0, 0, 0]
        );
    }

    #[test]
    fn test_temp_buffer_init_then_acquire_with_checked() {
        let mut data = [0u8; 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let detached = temp_buffer
            .init_prefix_with_checked(|buffer| {
                buffer[..2].write_copy_of_slice(&[21, 22]);
                Ok::<usize, TestError>(2)
            })
            .unwrap()
            .unwrap();

        assert_eq!(detached, &[21, 22]);
        assert_eq!(prefix_arena.len(), 3);
    }

    #[test]
    fn test_temp_buffer_init_then_acquire_with_error_preserves_arena() {
        let mut data = [1u8, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let error = temp_buffer
            .init_prefix_with(|buffer| {
                buffer[0].write(99);
                Err::<usize, TestError>(TestError::Expected)
            })
            .unwrap_err();

        assert_eq!(error, TestError::Expected);
        assert_eq!(prefix_arena.len(), 5);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[99, 2, 3, 4, 5]
        );
    }

    #[test]
    fn test_temp_buffer_init_then_acquire_with_checked_error_preserves_arena() {
        let mut data = [1u8, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let error = temp_buffer
            .init_prefix_with_checked(|buffer| {
                buffer[0].write(99);
                Err::<usize, TestError>(TestError::Expected)
            })
            .unwrap_err();

        assert_eq!(error, TestError::Expected);
        assert_eq!(prefix_arena.len(), 5);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[99, 2, 3, 4, 5]
        );
    }

    #[test]
    fn test_temp_buffer_init_then_acquire_with_checked_invalid_len_preserves_arena() {
        let mut data = [1u8, 2, 3];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let detached = temp_buffer
            .init_prefix_with_checked(|buffer| {
                buffer[0].write(9);
                Ok::<usize, TestError>(4)
            })
            .unwrap();

        assert_eq!(detached, None);
        assert_eq!(prefix_arena.len(), 3);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[9, 2, 3]
        );
    }

    #[test]
    #[should_panic(
        expected = "Initializer function returned a length greater than the current buffer size"
    )]
    fn test_temp_buffer_as_mut_with_init_panics_on_invalid_len() {
        let mut data = [0u8; 3];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let mut temp_buffer = prefix_arena.view();

        let _ = temp_buffer.init_with(|_| Ok::<usize, TestError>(4));
    }

    #[test]
    #[should_panic(
        expected = "Initializer function returned a length greater than the current buffer size"
    )]
    fn test_temp_buffer_init_then_acquire_with_panics_on_invalid_len() {
        let mut data = [0u8; 3];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let _ = temp_buffer.init_prefix_with(|_| Ok::<usize, TestError>(4));
    }

    #[test]
    fn test_take_front_partial() {
        let mut data = [1, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = unsafe { prefix_arena.take_prefix_unchecked(2) };
        assert_eq!(detached, &[1, 2]);
        assert_eq!(prefix_arena.len(), 3);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[3, 4, 5]
        );
    }

    #[test]
    fn test_take_front_all() {
        let mut data = [1, 2, 3];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = unsafe { prefix_arena.take_prefix_unchecked(3) };
        assert_eq!(detached, &[1, 2, 3]);
        assert_eq!(prefix_arena.len(), 0);
        assert!(prefix_arena.is_empty());
    }

    #[test]
    fn test_multiple_take_fronts() {
        let mut data = [1, 2, 3, 4, 5, 6];
        let prefix_arena = PrefixArena::new(&mut data);
        let first = unsafe { prefix_arena.take_prefix_unchecked(2) };
        assert_eq!(first, &[1, 2]);
        assert_eq!(prefix_arena.len(), 4);

        let second = unsafe { prefix_arena.take_prefix_unchecked(2) };
        assert_eq!(second, &[3, 4]);
        assert_eq!(prefix_arena.len(), 2);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[5, 6]
        );
    }

    #[test]
    #[should_panic]
    fn test_take_front_too_large() {
        let mut data = [1, 2, 3];
        let prefix_arena = PrefixArena::new(&mut data);
        prefix_arena.take_prefix(4);
    }

    #[test]
    fn test_take_front_checked() {
        let mut data = [1, 2, 3];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = prefix_arena.take_prefix_checked(2).unwrap();
        assert_eq!(unsafe { assume_init_as_bytes(detached) }, &[1, 2]);
        assert_eq!(prefix_arena.len(), 1);
    }

    #[test]
    fn test_take_front_checked_too_large_preserves_arena() {
        let mut data = [1, 2, 3];
        let prefix_arena = PrefixArena::new(&mut data);

        assert!(prefix_arena.take_prefix_checked(4).is_none());
        assert_eq!(prefix_arena.len(), 3);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[1, 2, 3]
        );
    }

    #[test]
    fn test_temp_buffer_take_prefix_shrinks_underlying_arena() {
        let mut data = [1, 2, 3, 4, 5];
        let mut prefix_arena = PrefixArena::new(&mut data);
        let temp_buffer = prefix_arena.view();

        let detached = temp_buffer.take_prefix(2);

        assert_eq!(unsafe { assume_init_as_bytes(detached) }, &[1, 2]);
        assert_eq!(prefix_arena.len(), 3);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[3, 4, 5]
        );
    }

    #[test]
    fn test_take_front_zero() {
        let mut data = [1, 2, 3];
        let prefix_arena = PrefixArena::new(&mut data);

        let detached = prefix_arena.take_prefix(0);
        assert_eq!(detached.len(), 0);
        assert_eq!(prefix_arena.len(), 3);
        assert_eq!(
            unsafe { assume_init_as_bytes(prefix_arena.take_remaining()) },
            &[1, 2, 3]
        );
    }

    #[test]
    fn test_usage_in_a_loop() {
        const PART_SIZE: usize = 3;
        const BUFFER_SIZE: usize = 10;
        const EXPECTED_PARTS: [u8; BUFFER_SIZE] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let mut data: [u8; BUFFER_SIZE] = EXPECTED_PARTS;

        let prefix_arena = PrefixArena::new(&mut data);

        let mut detached_parts = Vec::new();

        while !prefix_arena.is_empty() {
            let to_detach = core::cmp::min(prefix_arena.len(), PART_SIZE);
            let detached = unsafe { prefix_arena.take_prefix_unchecked(to_detach) };
            detached_parts.push(detached);
        }

        let mut expected_it = EXPECTED_PARTS.iter();
        for detached in detached_parts {
            detached.iter().for_each(|&byte| {
                assert_eq!(byte, *expected_it.next().unwrap());
            });
        }
    }

    #[test]
    fn test_no_borrowing_glue() {
        const PART_SIZE: usize = 3;
        const BUFFER_SIZE: usize = 10;
        const EXPECTED_PARTS: [u8; BUFFER_SIZE] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let mut data = EXPECTED_PARTS;
        let prefix_arena = PrefixArena::new(&mut data);

        let mut detached_parts = Vec::new();

        while !prefix_arena.is_empty() {
            let to_detach = core::cmp::min(prefix_arena.len(), PART_SIZE);
            let detached = unsafe { prefix_arena.take_prefix_unchecked(to_detach) };
            detached_parts.push(detached);
        }

        let mut expected_it = EXPECTED_PARTS.iter();
        for detached in detached_parts {
            detached.iter().for_each(|&byte| {
                assert_eq!(byte, *expected_it.next().unwrap());
            });
        }
    }

    #[test]
    fn test_no_borrowing_glue_with_temp_buffer_detach() {
        const PART_SIZE: usize = 3;
        const BUFFER_SIZE: usize = 10;
        const EXPECTED_PARTS: [u8; BUFFER_SIZE] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let mut data = EXPECTED_PARTS;
        let mut prefix_arena = PrefixArena::new(&mut data);

        let mut detached_parts = Vec::new();

        while !prefix_arena.is_empty() {
            let temp_buffer = prefix_arena.view();
            let to_detach = core::cmp::min(temp_buffer.len(), PART_SIZE);
            let detached = unsafe { temp_buffer.take_prefix_unchecked(to_detach) };
            detached_parts.push(detached);
        }

        let mut expected_it = EXPECTED_PARTS.iter();
        for detached in detached_parts {
            detached.iter().for_each(|&byte| {
                assert_eq!(byte, *expected_it.next().unwrap());
            });
        }
    }

    /// Test take_remaining method
    #[test]
    fn test_take_remaining() {
        let mut data = [1, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::new(&mut data);
        let remaining = unsafe { assume_init_as_bytes_mut(prefix_arena.take_remaining()) };
        assert_eq!(remaining, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn subsequent_take_remaining() {
        let mut data = [1, 2, 3, 4, 5];
        let prefix_arena = PrefixArena::new(&mut data);
        let _ = prefix_arena.take_prefix(2); // Detach first 2 bytes
        let remaining = unsafe { assume_init_as_bytes_mut(prefix_arena.take_remaining()) };
        assert_eq!(remaining, &[3, 4, 5]);
    }
}
