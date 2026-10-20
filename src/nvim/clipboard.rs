//! Clipboard register handling (`src/nvim/clipboard.c`).

#![cfg_attr(test, allow(unused_mut))]
#![allow(non_snake_case, clippy::missing_safety_doc)]

#[cfg(not(test))]
use crate::eval::typval::{tv_list_alloc, tv_list_append_list, tv_list_append_string};
#[cfg(not(test))]
use crate::eval::{eval_call_provider, eval_has_provider};
use crate::map_glyph_cache::NvimString;
#[cfg(not(test))]
use crate::mcursor::mc_replaying;
use crate::memory::xstrdup;
#[cfg(not(test))]
use crate::memory::{xcalloc, xfree};
#[cfg(not(test))]
use crate::message::{emsg, msg, redirecting};
#[cfg(not(test))]
use crate::option_vars::*;
#[cfg(not(test))]
use crate::register::{free_register, get_y_previous, get_y_register, update_yankreg_width};
#[cfg(not(test))]
use std::ffi::c_uint;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

#[cfg(not(test))]
const STAR_REGISTER: c_int = 37;
#[cfg(not(test))]
const PLUS_REGISTER: c_int = 38;

#[cfg(not(test))]
const K_OPT_CB_FLAG_UNNAMED: c_uint = 0x01;
#[cfg(not(test))]
const K_OPT_CB_FLAG_UNNAMEDPLUS: c_uint = 0x02;

#[cfg(not(test))]
const CTRL_V: u8 = 0x16;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum MotionType {
    kMTUnknown = -1,
    kMTCharWise = 0,
    kMTLineWise = 1,
    kMTBlockWise = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum VarType {
    VAR_UNKNOWN = 0,
    VAR_NUMBER = 1,
    VAR_STRING = 2,
    VAR_FUNC = 3,
    VAR_LIST = 4,
    VAR_DICT = 5,
    VAR_FLOAT = 6,
    VAR_BOOL = 7,
    VAR_SPECIAL = 8,
    VAR_PARTIAL = 9,
    VAR_BLOB = 10,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub union TypvalVval {
    pub v_number: i64,
    pub v_bool: c_int,
    pub v_special: c_int,
    pub v_float: f64,
    pub v_string: *mut c_char,
    pub v_list: *mut list_T,
    pub v_dict: *mut crate::eval::decode::dict_T,
    pub v_blob: *mut crate::eval::executor::blob_T,
    pub v_ptr: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct typval_T {
    pub v_type: VarType,
    pub v_lock: c_int,
    pub vval: TypvalVval,
}

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct listitem_T {
    pub li_next: *mut listitem_T,
    pub li_prev: *mut listitem_T,
    pub li_tv: typval_T,
}

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct list_T {
    pub lv_first: *mut listitem_T,
    pub lv_last: *mut listitem_T,
    pub lv_watch: *mut c_void,
    pub lv_idx_item: *mut listitem_T,
    pub lv_copylist: *mut list_T,
    pub lv_used_next: *mut list_T,
    pub lv_used_prev: *mut list_T,
    pub lv_refcount: c_int,
    pub lv_len: c_int,
    pub lv_idx: c_int,
    pub lv_copyID: c_int,
    pub lv_lock: c_int,
    pub lua_table_ref: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub struct yankreg_T {
    pub y_array: *mut NvimString,
    pub y_size: usize,
    pub y_type: MotionType,
    pub y_width: c_int,
    pub timestamp: u64,
    pub additional_data: *mut c_void,
}

static BATCH_CHANGE_COUNT: AtomicI32 = AtomicI32::new(0);
static CLIPBOARD_DELAY_UPDATE: AtomicBool = AtomicBool::new(false);
static CLIPBOARD_NEEDS_UPDATE: AtomicBool = AtomicBool::new(false);
#[cfg(not(test))]
static CLIPBOARD_DIDWARN: AtomicBool = AtomicBool::new(false);

const MSG_NO_CLIP: &CStr = c"clipboard: No provider. Try \":checkhealth\" or \":h clipboard\".";

#[inline]
unsafe fn cstr_to_nvim_string(s: *const c_char) -> NvimString {
    let ptr = if s.is_null() { c"".as_ptr() } else { s };
    let len = unsafe { CStr::from_ptr(ptr).to_bytes().len() };
    let data = unsafe { xstrdup(ptr) };
    NvimString { data, size: len }
}

/// Determine if register `*name` should be used as a clipboard (`adjust_clipboard_name`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn adjust_clipboard_name(
    name: *mut c_int,
    quiet: bool,
    writing: bool,
) -> *mut yankreg_T {
    #[cfg(test)]
    {
        let _ = (
            name,
            quiet,
            writing,
            MSG_NO_CLIP,
            cstr_to_nvim_string as unsafe fn(*const c_char) -> NvimString,
        );
        std::ptr::null_mut()
    }

    #[cfg(not(test))]
    unsafe {
        let reg_char = *name;
        let flags = cb_flags;
        let explicit_cb_reg = reg_char == b'*' as c_int || reg_char == b'+' as c_int;
        let implicit_cb_reg =
            reg_char == 0 && (flags & (K_OPT_CB_FLAG_UNNAMED | K_OPT_CB_FLAG_UNNAMEDPLUS)) != 0;
        if !explicit_cb_reg && !implicit_cb_reg {
            return std::ptr::null_mut();
        }

        if !eval_has_provider(c"clipboard".as_ptr(), false) {
            if BATCH_CHANGE_COUNT.load(Ordering::Relaxed) <= 1
                && !quiet
                && (!CLIPBOARD_DIDWARN.load(Ordering::Relaxed)
                    || (explicit_cb_reg && redirecting() == 0))
            {
                CLIPBOARD_DIDWARN.store(true, Ordering::Relaxed);
                msg(MSG_NO_CLIP.as_ptr(), 0);
            }
            return std::ptr::null_mut();
        }

        if explicit_cb_reg {
            let target = get_y_register(if reg_char == b'*' as c_int {
                STAR_REGISTER
            } else {
                PLUS_REGISTER
            });
            let match_flag = if reg_char == b'*' as c_int {
                K_OPT_CB_FLAG_UNNAMED
            } else {
                K_OPT_CB_FLAG_UNNAMEDPLUS
            };
            if writing && (flags & match_flag) != 0 {
                CLIPBOARD_NEEDS_UPDATE.store(false, Ordering::Relaxed);
            }
            target
        } else {
            if writing && CLIPBOARD_DELAY_UPDATE.load(Ordering::Relaxed) {
                CLIPBOARD_NEEDS_UPDATE.store(true, Ordering::Relaxed);
                return std::ptr::null_mut();
            } else if !writing && CLIPBOARD_NEEDS_UPDATE.load(Ordering::Relaxed) {
                return std::ptr::null_mut();
            }

            if (flags & K_OPT_CB_FLAG_UNNAMEDPLUS) != 0 {
                *name = if (flags & K_OPT_CB_FLAG_UNNAMED) != 0 && writing {
                    b'"' as c_int
                } else {
                    b'+' as c_int
                };
                get_y_register(PLUS_REGISTER)
            } else {
                *name = b'*' as c_int;
                get_y_register(STAR_REGISTER)
            }
        }
    }
}

/// Read clipboard contents into `*target` (`get_clipboard`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_clipboard(
    mut name: c_int,
    target: *mut *mut yankreg_T,
    quiet: bool,
) -> bool {
    #[cfg(test)]
    {
        let _ = (name, target, quiet);
        false
    }

    #[cfg(not(test))]
    unsafe {
        let implicit = name == 0;
        let mut errmsg = true;

        let reg_ptr = adjust_clipboard_name(&mut name, quiet, false);
        if reg_ptr.is_null() {
            return false;
        }
        if mc_replaying() {
            if implicit {
                return false;
            }
            *target = reg_ptr;
            return true;
        }
        free_register(reg_ptr);
        let reg = &mut *reg_ptr;

        let args = tv_list_alloc(1);
        let regname = name as c_char;
        tv_list_append_string(args, &regname, 1);

        let result = eval_call_provider(
            c"clipboard".as_ptr().cast_mut(),
            c"get".as_ptr().cast_mut(),
            args,
            false,
        );

        let fail_cleanup =
            |reg: &mut yankreg_T, errmsg: bool, target: *mut *mut yankreg_T| -> bool {
                if !reg.y_array.is_null() {
                    for i in 0..reg.y_size {
                        xfree((*reg.y_array.add(i)).data.cast::<c_void>());
                    }
                    xfree(reg.y_array.cast::<c_void>());
                }
                reg.y_array = std::ptr::null_mut();
                reg.y_size = 0;
                reg.additional_data = std::ptr::null_mut();
                reg.timestamp = 0;
                if errmsg {
                    emsg(c"clipboard: provider returned invalid data".as_ptr());
                }
                *target = reg as *mut yankreg_T;
                false
            };

        if result.v_type != VarType::VAR_LIST {
            if result.v_type == VarType::VAR_NUMBER && result.vval.v_number == 0 {
                errmsg = false;
            }
            return fail_cleanup(reg, errmsg, target);
        }

        let res = result.vval.v_list;
        let res_len = if res.is_null() { 0 } else { (*res).lv_len };
        let lines: *mut list_T;

        if res_len == 2
            && !(*res).lv_first.is_null()
            && (*(*res).lv_first).li_tv.v_type == VarType::VAR_LIST
        {
            lines = (*(*res).lv_first).li_tv.vval.v_list;
            let last_li = (*res).lv_last;
            if last_li.is_null() || (*last_li).li_tv.v_type != VarType::VAR_STRING {
                return fail_cleanup(reg, errmsg, target);
            }
            let regtype = (*last_li).li_tv.vval.v_string;
            if regtype.is_null() || CStr::from_ptr(regtype).to_bytes().len() > 1 {
                return fail_cleanup(reg, errmsg, target);
            }
            reg.y_type = match *regtype as u8 {
                0 => MotionType::kMTUnknown,
                b'v' | b'c' => MotionType::kMTCharWise,
                b'V' | b'l' => MotionType::kMTLineWise,
                b'b' | CTRL_V => MotionType::kMTBlockWise,
                _ => return fail_cleanup(reg, errmsg, target),
            };
        } else {
            lines = res;
            reg.y_type = MotionType::kMTUnknown;
        }

        let lines_len = if lines.is_null() {
            0
        } else {
            (*lines).lv_len.max(0) as usize
        };
        reg.y_array = xcalloc(lines_len, std::mem::size_of::<NvimString>()).cast::<NvimString>();
        reg.y_size = lines_len;
        reg.y_width = 0;
        reg.additional_data = std::ptr::null_mut();
        reg.timestamp = 0;

        let mut tv_idx = 0usize;
        let mut li = if lines.is_null() {
            std::ptr::null_mut()
        } else {
            (*lines).lv_first
        };
        while !li.is_null() {
            if (*li).li_tv.v_type != VarType::VAR_STRING {
                return fail_cleanup(reg, errmsg, target);
            }
            let s = (*li).li_tv.vval.v_string;
            *reg.y_array.add(tv_idx) = cstr_to_nvim_string(s);
            tv_idx += 1;
            li = (*li).li_next;
        }

        if reg.y_size > 0 && (*reg.y_array.add(reg.y_size - 1)).size == 0 {
            if reg.y_type != MotionType::kMTCharWise {
                xfree((*reg.y_array.add(reg.y_size - 1)).data.cast::<c_void>());
                reg.y_size -= 1;
                if reg.y_type == MotionType::kMTUnknown {
                    reg.y_type = MotionType::kMTLineWise;
                }
            }
        } else if reg.y_type == MotionType::kMTUnknown {
            reg.y_type = MotionType::kMTCharWise;
        }

        update_yankreg_width(reg_ptr);
        *target = reg_ptr;
        true
    }
}

/// Write `reg` to the clipboard provider (`set_clipboard`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn set_clipboard(mut name: c_int, reg: *mut yankreg_T) {
    #[cfg(test)]
    {
        let _ = (name, reg);
    }

    #[cfg(not(test))]
    unsafe {
        if mc_replaying() || adjust_clipboard_name(&mut name, false, true).is_null() {
            return;
        }

        let reg = &*reg;
        let extra = usize::from(reg.y_type != MotionType::kMTCharWise);
        let lines = tv_list_alloc((reg.y_size + extra) as isize);

        for i in 0..reg.y_size {
            let item = *reg.y_array.add(i);
            tv_list_append_string(lines, item.data, item.size as isize);
        }

        let regtype: c_char = match reg.y_type {
            MotionType::kMTLineWise => {
                tv_list_append_string(lines, std::ptr::null(), 0);
                b'V' as c_char
            }
            MotionType::kMTCharWise => b'v' as c_char,
            MotionType::kMTBlockWise => {
                tv_list_append_string(lines, std::ptr::null(), 0);
                b'b' as c_char
            }
            MotionType::kMTUnknown => std::process::abort(),
        };

        let args = tv_list_alloc(3);
        tv_list_append_list(args, lines);
        tv_list_append_string(args, &regtype, 1);
        let name_char = name as c_char;
        tv_list_append_string(args, &name_char, 1);

        let _ = eval_call_provider(
            c"clipboard".as_ptr().cast_mut(),
            c"set".as_ptr().cast_mut(),
            args,
            true,
        );
    }
}

/// Defer clipboard updates during batch operations (`start_batch_changes`).
#[unsafe(no_mangle)]
pub extern "C" fn start_batch_changes() {
    if BATCH_CHANGE_COUNT.fetch_add(1, Ordering::Relaxed) + 1 > 1 {
        return;
    }
    CLIPBOARD_DELAY_UPDATE.store(true, Ordering::Relaxed);
}

/// End batch operation and flush deferred clipboard update if needed (`end_batch_changes`).
#[unsafe(no_mangle)]
pub extern "C" fn end_batch_changes() {
    if BATCH_CHANGE_COUNT.fetch_sub(1, Ordering::Relaxed) - 1 > 0 {
        return;
    }
    CLIPBOARD_DELAY_UPDATE.store(false, Ordering::Relaxed);
    if CLIPBOARD_NEEDS_UPDATE.swap(false, Ordering::Relaxed) {
        #[cfg(not(test))]
        unsafe {
            set_clipboard(0, get_y_previous());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_changes_nesting_toggles_delay_flag() {
        start_batch_changes();
        assert!(CLIPBOARD_DELAY_UPDATE.load(Ordering::Relaxed));
        start_batch_changes();
        end_batch_changes();
        assert!(CLIPBOARD_DELAY_UPDATE.load(Ordering::Relaxed));
        end_batch_changes();
        assert!(!CLIPBOARD_DELAY_UPDATE.load(Ordering::Relaxed));
    }
}
