#![deny(unsafe_code)]

pub mod base64;
pub mod converter;
pub mod executor;
#[allow(unsafe_code)]
pub mod ffi;
pub mod secure;
pub mod spell;
pub mod stdlib;
pub mod treesitter;
pub mod xdiff;
