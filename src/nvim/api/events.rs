//! API event handlers (`src/nvim/api/events.c`).
#![forbid(unsafe_code)]

use crate::api::private::helpers::api_typename;
use crate::api::private::validate::{Error, NvimString, Object, ObjectType, api_err_exp};
use crate::autocmd::do_termresponse_autocmd;
use crate::eval::vars::{VV_TERMRESPONSE, set_vim_var_bytes};
use crate::log::{LOGLVL_ERR, logmsg};

/// Emitted on the client channel if an async API request responds with an error.
///
/// @param channel_id
/// @param type Error type id as defined by `api_info().error_types`.
/// @param msg Error message.
// DLLEXPORT void nvim_error_event(uint64_t channel_id, Integer type, String msg) FUNC_API_REMOTE_ONLY;
pub fn nvim_error_event(channel_id: u64, _type: i64, msg: NvimString) {
    let msg_cstr = if msg.size != 0 { msg.as_cstr() } else { c"" };
    logmsg!(
        LOGLVL_ERR,
        None,
        Some(c"nvim_error_event"),
        43,
        true,
        c"async error on channel %lld: %s",
        channel_id as i64,
        msg_cstr,
    );
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
pub fn nvim_ui_term_event(channel_id: u64, event: NvimString, value: Object) -> Result<(), Error> {
    if !event.data.is_null() && event.cstr_bytes() == b"termresponse" {
        if value.obj_type != ObjectType::ObjectTypeString {
            return Err(api_err_exp(
                c"termresponse",
                api_typename(ObjectType::ObjectTypeString),
                Some(api_typename(value.obj_type)),
            ));
        }

        let termresponse = value.data.string();
        set_vim_var_bytes(VV_TERMRESPONSE, termresponse.as_bytes());
        do_termresponse_autocmd(termresponse, channel_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_event_noop_in_unit_test() {
        let msg = NvimString::new();
        nvim_error_event(1, 0, msg);
    }
}
