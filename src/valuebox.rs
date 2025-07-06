use crossbeam_utils::CachePadded;
use std::{cell::UnsafeCell, mem::MaybeUninit};

pub struct ValueBox<T: Sized + Clone>(CachePadded<UnsafeCell<MaybeUninit<T>>>);

impl<T: Sized + Clone> ValueBox<T> {
    pub fn uninit() -> Self {
        Self(CachePadded::new(UnsafeCell::new(MaybeUninit::uninit())))
    }

    pub unsafe fn get(&self) -> &T {
        unsafe {
            let m = &*self.0.get();
            m.assume_init_ref()
        }
    }

    pub unsafe fn write(&self, value: T) {
        unsafe {
            let m = &mut *self.0.get();
            m.write(value);
        }
    }
}
