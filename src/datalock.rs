/***************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **************************************************************************************************/

//! # Data Lock
//!
//! Enable exclusive access to data guarded by a cross core atomic lock. In contrast to a ``Singleton``
//! the data access lock could also be non-blocking and might fail. But exclusive access is guaranteed
//! across cores if the lock could be aquired.
//!
//! # Example
//! ```
//! use ruspiro_lock::DataLock;
//!
//! static DATA: DataLock<u32> = DataLock::new(0);
//!
//! fn main() {
//!     if let Some(mut data) = DATA.try_lock() {
//!         *data = 20;
//!     }
//!     // once the data goes ot of scope the lock will be released
//!     if let Some(data) = DATA.try_lock() {
//!         println!("data: {}", *data);
//!
//!         // another lock should fail inside this scope
//!         assert!(DATA.try_lock().is_none());
//!     }
//! }
//! ```
//! This example uses a ``static`` variable to define a lock that shall be available across cores.
//! The data might also be wrapped in an ``Arc<DataLock<T>>`` and shared between cores using clones
//! of the ``Arc``.
//!

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// An exclusive access lock around the given data
#[repr(C, align(16))]
pub struct DataLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

/// Result of trying to access the data using ``try_lock`` or ``lock`` on the data lock. If the
/// result goes out of scope the lock is released.
#[derive(Debug)]
pub struct TryDataLock<'a, T> {
    _data: &'a DataLock<T>,
}

impl<T> DataLock<T> {
    /// Create a new data access guarding lock
    pub const fn new(value: T) -> Self {
        DataLock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    /// Try to lock the guarded data for mutual exclusive access. Returns ``None`` if the lock failes
    /// or ``Some(TryDataLock)``. The actual data, the TryDataLock wraps could be conviniently accessed by
    /// dereferencing it.
    ///
    /// # Example
    /// ```
    /// # use ruspiro_lock::DataLock;
    /// static DATA: DataLock<u32> = DataLock::new(10);
    /// # fn main() {
    ///     if let Some(data) = DATA.try_lock() {
    ///         // do something with data
    ///     }
    /// # }
    /// ```
    pub fn try_lock(&self) -> Option<TryDataLock<T>> {
        // do the atomic operation to set the lock
        if !self.locked.swap(true, Ordering::SeqCst) {
            // has been false previously means we now have the lock

            #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
            unsafe {
                // dmb required before allow access to the protected resource, see:
                // http://infocenter.arm.com/help/topic/com.arm.doc.dht0008a/DHT0008A_arm_synchronization_primitives.pdf
                llvm_asm!("dmb sy");
            }

            Some(TryDataLock { _data: self })
        } else {
            // we couldn't set the lock
            None
        }
    }

    /// Lock the guarded data for mutual exclusive access. This blocks until the data could be
    /// successfully locked. The locked data will be returned as ``TryDataLock``. Simply derefrencing
    /// this allows access to the contained data value.
    ///
    /// # Example
    /// ```
    /// # use ruspiro_lock::DataLock;
    /// static DATA: DataLock<u32> = DataLock::new(10);
    /// # fn main() {
    ///     let mut data = DATA.lock();
    ///     // do something with data
    ///     *data = 15;
    ///
    /// # }
    /// ```
    pub fn lock(&self) -> TryDataLock<T> {
        loop {
            if let Some(data) = self.try_lock() {
                return data;
            }
            // to save energy and cpu consumption we can wait for an event beeing raised that indicates that the 
            // semaphore value has likely beeing changed
            #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
            unsafe { llvm_asm!("wfe"); }
        }
    }
}

impl<T> core::fmt::Debug for DataLock<T> 
    where T: core::fmt::Debug
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DataLock")
            .field("Value", unsafe { &*self.data.get() })
            .finish()
    }
}

// when the TryDataLock is dropped release the owning lock
impl<T> Drop for TryDataLock<'_, T> {
    fn drop(&mut self) {
        self._data.locked.swap(false, Ordering::SeqCst);

        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        unsafe {
            // dmb required before allow access to the protected resource, see:
            // http://infocenter.arm.com/help/topic/com.arm.doc.dht0008a/DHT0008A_arm_synchronization_primitives.pdf
            llvm_asm!("dmb sy");
            // also raise a signal to indicate the semaphore has been changed (this trigger all WFE's to continue 
            // processing) but do data syncronisation barrier upfront to ensure any data updates has been finished
            llvm_asm!(
                "dsb sy
                 sev"
            );
        }
    }
}

// dereferencing the value contained in the TryDataLock
// this ok as the TryDataLock does only exist if the exclusive access to the data could
// be ensures. Therefore also only one ``TryDataLock`` could ever exist for one specific ``DataLock``, which makes it
// safe to return immutable and mutable references.
impl<T> Deref for TryDataLock<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self._data.data.get() }
    }
}

impl<T> DerefMut for TryDataLock<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self._data.data.get() }
    }
}

unsafe impl<T> Sync for DataLock<T> {}
unsafe impl<T> Send for DataLock<T> {}
