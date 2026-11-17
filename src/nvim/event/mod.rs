#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod ffi;
pub mod inbox;
#[allow(unsafe_code)]
pub mod libuv_proc;
#[allow(unsafe_code)]
pub mod r#loop;
#[allow(unsafe_code)]
pub mod multiqueue;
#[allow(unsafe_code)]
pub mod owned;
#[allow(unsafe_code)]
pub mod proc;
pub mod read_buffer;
#[allow(unsafe_code)]
pub mod rstream;
#[allow(unsafe_code)]
pub mod signal;
#[allow(unsafe_code)]
pub mod socket;
#[allow(unsafe_code)]
pub mod stream;
#[allow(unsafe_code)]
pub mod time;
#[cfg_attr(test, allow(unsafe_code))]
pub mod timer;
#[allow(unsafe_code)]
pub mod wstream;
