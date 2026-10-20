//! API event handlers (`src/nvim/api/events.c`).

#![allow(
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    clippy::empty_line_after_doc_comments,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

#[cfg(not(test))]
use crate::api::private::helpers::api_typename;
use crate::api::private::validate::{Error, Object};
#[cfg(not(test))]
use crate::api::private::validate::{ObjectType, api_err_exp};
#[cfg(not(test))]
use crate::autocmd::do_termresponse_autocmd;
#[cfg(not(test))]
use crate::eval::vars::set_vim_var_string;
#[cfg(not(test))]
use crate::log::logmsg;
use crate::map_glyph_cache::NvimString;
#[cfg(not(test))]
use std::ffi::CStr;
#[cfg(not(test))]
use std::ffi::c_int;

#[cfg(not(test))]
const VV_TERMRESPONSE: c_int = 11;
#[cfg(not(test))]
const LOGLVL_ERR: c_int = 4;

/// Emitted on the client channel if an async API request responds with an error.
///
/// @param channel_id
/// @param type Error type id as defined by `api_info().error_types`.
/// @param msg Error message.
// DLLEXPORT void nvim_error_event(uint64_t channel_id, Integer type, String msg) FUNC_API_REMOTE_ONLY;
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

/// Emitted by the TUI client to signal when a host-terminal event occurred.
///
/// Supports these events:
///
///   - "termresponse": The host-terminal sent an OSC, DCS, APC, or recognized CSI (DA1, kitty
///     multiple-cursors) response to Nvim. The payload is the received response. Sets
///     |v:termresponse| and fires |TermResponse|.
///
/// @param channel_id
/// @param event Event name
/// @param value Event payload
/// @param[out] err Error details, if any.
// DLLEXPORT void nvim_ui_term_event(uint64_t channel_id, String event, Object value,
//                                   Error *err) FUNC_API_SINCE(12) FUNC_API_REMOTE_ONLY;
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
