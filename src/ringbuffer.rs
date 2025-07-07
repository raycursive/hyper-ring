use std::{
    cell::Cell,
    marker::PhantomData,
    sync::atomic::{AtomicBool, AtomicUsize},
};

use crate::valuebox::ValueBox;

/**
 * Trait for ring buffer container.
 */
pub trait RBContainer<T: Sized + Clone>: Sized {
    fn retrieve<'a>(&'a self, cursor: usize) -> Option<&'a T>;
    fn put(&self, value: T);
    fn mark_as_finished(&self);
    fn is_finished(&self) -> bool;
    fn next_readable(&self) -> usize;
}

/// RingBuffer used for single producer scenario
pub struct SPRingBuffer<T: Sized + Clone, const N: usize> {
    data: [ValueBox<T>; N],
    next_write: Cell<usize>,
    next_readable: AtomicUsize,
    is_finished: AtomicBool,
}

impl<T: Sized + Clone, const N: usize> SPRingBuffer<T, N> {
    pub fn new() -> Self {
        Self {
            data: core::array::from_fn(|_| ValueBox::uninit()),
            next_write: Cell::new(0),
            next_readable: AtomicUsize::new(0),
            is_finished: AtomicBool::new(false),
        }
    }

    pub fn split(&self) -> (RBReader<T, Self>, RBWriter<T, N>) {
        (
            RBReader {
                ring_buffer: self,
                cursor: 0,
                _marker: PhantomData,
            },
            RBWriter {
                ring_buffer: self,
                finished: false,
            },
        )
    }
}

impl<T: Sized + Clone, const N: usize> RBContainer<T> for SPRingBuffer<T, N> {
    #[inline(always)]
    fn retrieve<'a>(&'a self, cursor: usize) -> Option<&'a T> {
        if cursor
            >= self
                .next_readable
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return None;
        }
        Some(unsafe { self.data.get_unchecked(cursor & (N - 1)).get() })
    }

    #[inline(always)]
    fn put(&self, value: T) {
        let curr_write_pos = self.next_write.get();
        self.next_write.set(curr_write_pos + 1);
        unsafe {
            self.data
                .get_unchecked(curr_write_pos & (N - 1))
                .write(value);
        }
        if cfg!(debug_assertions) {
            let write_pos_checker = self
                .next_readable
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            assert_eq!(write_pos_checker, curr_write_pos);
        } else {
            self.next_readable
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }
    }

    #[inline(always)]
    fn mark_as_finished(&self) {
        self.is_finished
            .store(true, std::sync::atomic::Ordering::Release);
    }

    #[inline(always)]
    fn is_finished(&self) -> bool {
        self.is_finished.load(std::sync::atomic::Ordering::Acquire)
    }

    #[inline(always)]
    fn next_readable(&self) -> usize {
        self.next_readable
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

/// RingBuffer used for multi producer scenario
pub struct MPRingBuffer<T: Sized + Clone, const N: usize> {
    data: [ValueBox<T>; N],
    next_write: AtomicUsize,
    next_readable: AtomicUsize,
    producer_count: AtomicUsize,
    finish_count: AtomicUsize,
}
impl<T: Sized + Clone, const N: usize> MPRingBuffer<T, N> {
    pub fn new() -> Self {
        Self {
            data: core::array::from_fn(|_| ValueBox::uninit()),
            next_write: AtomicUsize::new(0),
            next_readable: AtomicUsize::new(0),
            producer_count: AtomicUsize::new(0),
            finish_count: AtomicUsize::new(0),
        }
    }

    pub fn split(&self) -> (RBReader<T, Self>, RBWriterClonable<T, N>) {
        self.producer_count
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        (
            RBReader {
                ring_buffer: self,
                cursor: 0,
                _marker: PhantomData,
            },
            RBWriterClonable {
                ring_buffer: self,
                finished: false,
            },
        )
    }
}

impl<T: Sized + Clone, const N: usize> RBContainer<T> for MPRingBuffer<T, N> {
    #[inline(always)]
    fn retrieve<'a>(&'a self, cursor: usize) -> Option<&'a T> {
        if cursor
            >= self
                .next_readable
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return None;
        }
        Some(unsafe { self.data.get_unchecked(cursor & (N - 1)).get() })
    }

    #[inline(always)]
    fn put(&self, value: T) {
        let mut curr_write_pos = self.next_write.load(std::sync::atomic::Ordering::Acquire);
        let mut next_write_pos = curr_write_pos.saturating_add(1);
        while let Err(e) = self.next_write.compare_exchange(
            curr_write_pos,
            next_write_pos,
            std::sync::atomic::Ordering::Release,
            std::sync::atomic::Ordering::Acquire,
        ) {
            curr_write_pos = e;
            next_write_pos = curr_write_pos.saturating_add(1);
        }
        unsafe {
            self.data
                .get_unchecked(curr_write_pos & (N - 1))
                .write(value);
        };
        self.next_readable
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    #[inline(always)]
    fn mark_as_finished(&self) {
        self.finish_count
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    #[inline(always)]
    fn is_finished(&self) -> bool {
        let producer_count = self
            .producer_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let finish_count = self.finish_count.load(std::sync::atomic::Ordering::Acquire);
        finish_count == producer_count
    }

    #[inline(always)]
    fn next_readable(&self) -> usize {
        self.next_readable
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

/// RingBuffer reader, cloneable, supports multiple concurrent readers maintaining their own cursor
pub struct RBReader<'a, T: Sized + Clone, C: RBContainer<T>> {
    ring_buffer: &'a C,
    cursor: usize,
    _marker: PhantomData<(T, C)>,
}

unsafe impl<'a, T: Sized + Clone, C: RBContainer<T>> Send for RBReader<'a, T, C> {}

impl<'a, T: Sized + Clone, C: RBContainer<T>> Clone for RBReader<'a, T, C> {
    fn clone(&self) -> Self {
        Self {
            ring_buffer: self.ring_buffer,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}
impl<'a, T: Sized + Clone, C: RBContainer<T>> RBReader<'a, T, C> {
    #[inline(always)]
    pub fn next(&mut self) -> Option<&T> {
        self.ring_buffer.retrieve(self.cursor).map(|v| {
            self.cursor += 1;
            v
        })
    }

    #[inline(always)]
    pub fn is_finished(&self) -> bool {
        self.ring_buffer.is_finished() && self.cursor == self.ring_buffer.next_readable()
    }
}

/// RingBuffer writer, non-cloneable, supports single writer
pub struct RBWriter<'a, T: Sized + Clone, const N: usize> {
    ring_buffer: &'a SPRingBuffer<T, N>,
    finished: bool,
}

unsafe impl<'a, T: Sized + Clone, const N: usize> Send for RBWriter<'a, T, N> {}

impl<'a, T: Sized + Clone, const N: usize> RBWriter<'a, T, N> {
    #[inline(always)]
    pub fn put(&mut self, value: T) {
        self.ring_buffer.put(value);
    }

    #[inline(always)]
    pub fn mark_as_finished(&mut self) {
        if !self.finished {
            self.finished = true;
            self.ring_buffer.mark_as_finished();
        }
    }
}

impl<T: Sized + Clone, const N: usize> Drop for RBWriter<'_, T, N> {
    fn drop(&mut self) {
        self.mark_as_finished();
    }
}

/// RingBuffer writer, cloneable, supports multiple concurrent writers
pub struct RBWriterClonable<'a, T: Sized + Clone, const N: usize> {
    ring_buffer: &'a MPRingBuffer<T, N>,
    finished: bool,
}

unsafe impl<'a, T: Sized + Clone, const N: usize> Send for RBWriterClonable<'a, T, N> {}

impl<'a, T: Sized + Clone, const N: usize> RBWriterClonable<'a, T, N> {
    #[inline(always)]
    pub fn put(&mut self, value: T) {
        self.ring_buffer.put(value);
    }

    #[inline(always)]
    pub fn mark_as_finished(&mut self) {
        if !self.finished {
            self.finished = true;
            self.ring_buffer.mark_as_finished();
        }
    }
}

impl<T: Sized + Clone, const N: usize> Drop for RBWriterClonable<'_, T, N> {
    fn drop(&mut self) {
        self.mark_as_finished();
    }
}

impl<T: Sized + Clone, const N: usize> Clone for RBWriterClonable<'_, T, N> {
    fn clone(&self) -> Self {
        self.ring_buffer
            .producer_count
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Self {
            ring_buffer: self.ring_buffer,
            finished: false,
        }
    }
}
