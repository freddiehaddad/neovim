//! API event handlers (`src/nvim/api/events.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clippy::missing_safety_doc
)]

#[allow(unused_imports)]
use crate::api::private::helpers::api_typename;
#[allow(unused_imports)]
use crate::autocmd::do_termresponse_autocmd;
#[allow(unused_imports)]
use crate::eval::vars::set_vim_var_string;
use crate::log::logmsg;
use std::ffi::{CStr, c_char, c_int};

use crate::api::private::validate::{Error, Object, ObjectType, api_err_exp};
use crate::map_glyph_cache::NvimString;

const VV_TERMRESPONSE: c_int = 11;
const LOGLVL_ERR: c_int = 3;

/// Emitted on the client channel if an async API request responds with an error (`nvim_error_event`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_error_event(channel_id: u64, _type: i64, msg: NvimString) {
    #[cfg(test)]
    let _ = (channel_id, msg);

    #[cfg(not(test))]
    unsafe {
        let msg_ptr = if msg.size != 0 {
            msg.data
        } else {
            c"".as_ptr()
        };
        logmsg!(
            LOGLVL_ERR,
            std::ptr::null(),
            c"nvim_error_event".as_ptr(),
            43,
            true,
            c"async error on channel %lld: %s".as_ptr(),
            channel_id as i64,
            msg_ptr,
        );
    }
}

/// Emitted by the TUI client to signal when a host-terminal event occurred (`nvim_ui_term_event`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_ui_term_event(
    channel_id: u64,
    event: NvimString,
    value: Object,
    err: *mut Error,
) {
    #[cfg(test)]
    let _ = (channel_id, event, value, err);

    #[cfg(not(test))]
    unsafe {
        if !event.data.is_null() && CStr::from_ptr(event.data).to_bytes() == b"termresponse" {
            if value.obj_type != ObjectType::kObjectTypeString {
                api_err_exp(
                    err,
                    c"termresponse".as_ptr(),
                    api_typename(ObjectType::kObjectTypeString),
                    api_typename(value.obj_type),
                );
                return;
            }

            let termresponse = value.data.string;
            set_vim_var_string(
                VV_TERMRESPONSE,
                termresponse.data,
                termresponse.size as isize,
            );
            do_termresponse_autocmd(termresponse, channel_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_event_noop_in_unit_test() {
        let msg = NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        };
        unsafe {
            nvim_error_event(1, 0, msg);
        }
    }
}
