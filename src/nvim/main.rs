#![no_main]

use std::ffi::{c_char, c_int};

#[unsafe(no_mangle)]
extern "C" fn main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    neovim::main_init::nvim_main(argc, argv)
}
