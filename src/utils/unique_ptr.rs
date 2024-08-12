use std::ops::{Deref, DerefMut};
use std::ptr;

/// UniquePtr is a smart pointer that owns and manages raw pointers and supports
/// custom deleter functions.
/// It's equivalent to std::unique_ptr in C++ and primarily intended to be used
/// in FFI bindings where we need to manage the memory of raw pointers.
//
/// Under the hood it uses Box<> to own the raw pointer, and implements the Drop
/// trait to call the custom deleter function when the UniquePtr is dropped.
//
/// Example:
//
/// ```rust
/// extern crate openssl::{SSL_new, SSL_free};
//
/// {
///     // Obtain pointer to SSL object and store it in UniquePtr alongside a deleter
///     // closure that free's the pointer when dropped
///     let ssl = UniquePtr::new(SSL_new(), |ptr| unsafe { SSL_free(ptr) });
//
///     // Access the wrapped pointer by dereferencing the UniquePtr
///     SSL_set_connect_state(*ssl);
/// } // SSL_free() is called here
pub struct UniquePtr<T, D = ()>
{
    ptr: *mut T,
    deleter: Box<unsafe extern "C" fn(*mut T) -> D>
}

impl<T, D> UniquePtr<T, D>
{

    /// Returns a new UniquePtr that takes ownership of the raw pointer `ptr` and
    /// uses the provided `deleter` function to free the pointer when the UniquePtr
    /// is dropped.
    pub fn new(ptr: *mut T, deleter: unsafe extern "C" fn(*mut T) -> D) -> Self {
        Self {
            ptr, deleter: Box::new(deleter)
        }
    }
}

impl<T, D> Deref for UniquePtr<T, D>
{
    type Target = *mut T;
    fn deref(&self) -> &Self::Target {
        return &self.ptr;
    }
}

impl<T, D> DerefMut for UniquePtr<T, D>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        return &mut self.ptr;
    }
}

impl<T, D> Drop for UniquePtr<T, D>
{
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                (self.deleter)(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}
