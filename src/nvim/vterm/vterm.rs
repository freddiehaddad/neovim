//! Core VTerm instance management and output buffering (`src/nvim/vterm/vterm.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc
)]

use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use crate::memory::{xfree, xmalloc};
use crate::vterm::mouse::VTerm;
use crate::vterm::parser::VTermParserCallbacks;
use crate::vterm::pen::{VTermAttr, VTermRect, VTermValueType};
use crate::vterm::screen::vterm_screen_free;
use crate::vterm::state::vterm_state_free;

pub type VTermOutputCallback =
    Option<unsafe extern "C" fn(s: *const c_char, len: usize, user: *mut c_void)>;

const NORMAL: c_int = 0;

const VT_MODE_UTF8: u32 = 1 << 0;
const VT_MODE_CTRL8BIT: u32 = 1 << 1;

const VTERM_VALUETYPE_BOOL: VTermValueType = 1;
const VTERM_VALUETYPE_INT: VTermValueType = 2;
const VTERM_VALUETYPE_STRING: VTermValueType = 3;
const VTERM_VALUETYPE_COLOR: VTermValueType = 4;

const VTERM_ATTR_BOLD: VTermAttr = 1;
const VTERM_ATTR_UNDERLINE: VTermAttr = 2;
const VTERM_ATTR_ITALIC: VTermAttr = 3;
const VTERM_ATTR_BLINK: VTermAttr = 4;
const VTERM_ATTR_REVERSE: VTermAttr = 5;
const VTERM_ATTR_CONCEAL: VTermAttr = 6;
const VTERM_ATTR_STRIKE: VTermAttr = 7;
const VTERM_ATTR_FONT: VTermAttr = 8;
const VTERM_ATTR_FOREGROUND: VTermAttr = 9;
const VTERM_ATTR_BACKGROUND: VTermAttr = 10;
const VTERM_ATTR_SMALL: VTermAttr = 11;
const VTERM_ATTR_BASELINE: VTermAttr = 12;
const VTERM_ATTR_URI: VTermAttr = 13;
const VTERM_ATTR_DIM: VTermAttr = 14;
const VTERM_ATTR_OVERLINE: VTermAttr = 15;
const VTERM_N_ATTRS: VTermAttr = 16;

#[repr(C)]
pub struct VTermAllocatorFunctions {
    pub malloc: Option<unsafe extern "C" fn(size: usize, allocdata: *mut c_void) -> *mut c_void>,
    pub free: Option<unsafe extern "C" fn(ptr: *mut c_void, allocdata: *mut c_void)>,
}

#[repr(C)]
pub struct VTermBuilder {
    pub ver: c_int,
    pub rows: c_int,
    pub cols: c_int,
    pub allocator: *const VTermAllocatorFunctions,
    pub allocdata: *mut c_void,
    pub outbuffer_len: usize,
    pub tmpbuffer_len: usize,
}

unsafe extern "C" fn default_malloc(size: usize, _allocdata: *mut c_void) -> *mut c_void {
    unsafe {
        let ptr = xmalloc(size);
        if !ptr.is_null() {
            ptr::write_bytes(ptr.cast::<u8>(), 0, size);
        }
        ptr
    }
}

unsafe extern "C" fn default_free(ptr: *mut c_void, _allocdata: *mut c_void) {
    unsafe {
        xfree(ptr);
    }
}

static DEFAULT_ALLOCATOR: VTermAllocatorFunctions = VTermAllocatorFunctions {
    malloc: Some(default_malloc),
    free: Some(default_free),
};

/// Convenient shortcut for default cases.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_new(rows: c_int, cols: c_int) -> *mut VTerm {
    unsafe {
        let builder = VTermBuilder {
            ver: 0,
            rows,
            cols,
            allocator: ptr::null(),
            allocdata: ptr::null_mut(),
            outbuffer_len: 0,
            tmpbuffer_len: 0,
        };
        vterm_build(&raw const builder)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_build(builder: *const VTermBuilder) -> *mut VTerm {
    unsafe {
        let allocator = if !(*builder).allocator.is_null() {
            (*builder).allocator
        } else {
            &raw const DEFAULT_ALLOCATOR
        };

        let malloc_fn = (*allocator).malloc.unwrap();
        let vt = malloc_fn(size_of::<VTerm>(), (*builder).allocdata).cast::<VTerm>();

        (*vt).allocator = allocator.cast();
        (*vt).allocdata = (*builder).allocdata;

        (*vt).rows = (*builder).rows;
        (*vt).cols = (*builder).cols;

        (*vt).parser.state = NORMAL;
        (*vt).parser.callbacks = ptr::null();
        (*vt).parser.cbdata = ptr::null_mut();
        (*vt).parser.emit_nul = false;

        (*vt).outfunc = ptr::null_mut();
        (*vt).outdata = ptr::null_mut();

        (*vt).outbuffer_len = if (*builder).outbuffer_len != 0 {
            (*builder).outbuffer_len
        } else {
            4096
        };
        (*vt).outbuffer_cur = 0;
        (*vt).outbuffer = vterm_allocator_malloc(vt, (*vt).outbuffer_len).cast::<c_char>();

        (*vt).tmpbuffer_len = if (*builder).tmpbuffer_len != 0 {
            (*builder).tmpbuffer_len
        } else {
            4096
        };
        (*vt).tmpbuffer = vterm_allocator_malloc(vt, (*vt).tmpbuffer_len).cast::<c_char>();

        vt
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_free(vt: *mut VTerm) {
    unsafe {
        if !(*vt).screen.is_null() {
            vterm_screen_free((*vt).screen.cast());
        }

        if !(*vt).state.is_null() {
            vterm_state_free((*vt).state.cast());
        }

        vterm_allocator_free(vt, (*vt).outbuffer.cast());
        vterm_allocator_free(vt, (*vt).tmpbuffer.cast());

        vterm_allocator_free(vt, vt.cast());
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_allocator_malloc(vt: *mut VTerm, size: usize) -> *mut c_void {
    unsafe {
        let allocator = (*vt).allocator.cast::<VTermAllocatorFunctions>();
        let malloc_fn = (*allocator).malloc.unwrap();
        malloc_fn(size, (*vt).allocdata)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_allocator_free(vt: *mut VTerm, ptr: *mut c_void) {
    unsafe {
        let allocator = (*vt).allocator.cast::<VTermAllocatorFunctions>();
        let free_fn = (*allocator).free.unwrap();
        free_fn(ptr, (*vt).allocdata);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_get_size(vt: *const VTerm, rowsp: *mut c_int, colsp: *mut c_int) {
    unsafe {
        if !rowsp.is_null() {
            *rowsp = (*vt).rows;
        }
        if !colsp.is_null() {
            *colsp = (*vt).cols;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_set_size(vt: *mut VTerm, rows: c_int, cols: c_int) {
    if rows < 1 || cols < 1 {
        return;
    }

    unsafe {
        (*vt).rows = rows;
        (*vt).cols = cols;

        let callbacks = (*vt).parser.callbacks.cast::<VTermParserCallbacks>();
        if !callbacks.is_null()
            && let Some(resize_cb) = (*callbacks).resize
        {
            let _ = resize_cb(rows, cols, (*vt).parser.cbdata);
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_set_utf8(vt: *mut VTerm, is_utf8: c_int) {
    unsafe {
        if is_utf8 != 0 {
            (*vt).mode |= VT_MODE_UTF8;
        } else {
            (*vt).mode &= !VT_MODE_UTF8;
        }
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_output_set_callback(
    vt: *mut VTerm,
    func: VTermOutputCallback,
    user: *mut c_void,
) {
    unsafe {
        (*vt).outfunc = match func {
            Some(f) => f as *mut c_void,
            None => ptr::null_mut(),
        };
        (*vt).outdata = user;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_push_output_bytes(vt: *mut VTerm, bytes: *const c_char, len: usize) {
    unsafe {
        if !(*vt).outfunc.is_null() {
            let func: unsafe extern "C" fn(*const c_char, usize, *mut c_void) =
                std::mem::transmute((*vt).outfunc);
            func(bytes, len, (*vt).outdata);
            return;
        }

        if len > (*vt).outbuffer_len - (*vt).outbuffer_cur {
            return;
        }

        ptr::copy_nonoverlapping(bytes, (*vt).outbuffer.add((*vt).outbuffer_cur), len);
        (*vt).outbuffer_cur += len;
    }
}

pub unsafe fn vterm_push_output_bytes_slice(vt: *mut VTerm, payload: &[u8]) {
    unsafe {
        if payload.len() >= (*vt).tmpbuffer_len {
            let len = payload.len();
            vterm_push_output_bytes(vt, (*vt).tmpbuffer, len);
            return;
        }
        ptr::copy_nonoverlapping(
            payload.as_ptr().cast::<c_char>(),
            (*vt).tmpbuffer,
            payload.len(),
        );
        *(*vt).tmpbuffer.add(payload.len()) = 0;
        vterm_push_output_bytes(vt, (*vt).tmpbuffer, payload.len());
    }
}

pub unsafe fn vterm_push_output_ctrl_bytes(vt: *mut VTerm, ctrl: u8, payload: &[u8]) {
    unsafe {
        let ctrl8bit = ((*vt).mode & VT_MODE_CTRL8BIT) != 0;

        let mut cur: usize = if ctrl >= 0x80 && !ctrl8bit {
            if (*vt).tmpbuffer_len <= 2 {
                return;
            }
            *(*vt).tmpbuffer = 0x1b as c_char;
            *(*vt).tmpbuffer.add(1) = (ctrl - 0x40) as c_char;
            2
        } else {
            if (*vt).tmpbuffer_len <= 1 {
                return;
            }
            *(*vt).tmpbuffer = ctrl as c_char;
            1
        };

        if cur + payload.len() >= (*vt).tmpbuffer_len {
            return;
        }

        ptr::copy_nonoverlapping(
            payload.as_ptr().cast::<c_char>(),
            (*vt).tmpbuffer.add(cur),
            payload.len(),
        );
        cur += payload.len();
        *(*vt).tmpbuffer.add(cur) = 0;

        vterm_push_output_bytes(vt, (*vt).tmpbuffer, cur);
    }
}

pub unsafe fn vterm_push_output_str_bytes(vt: *mut VTerm, ctrl: u8, term: bool, payload: &[u8]) {
    unsafe {
        let mut cur: usize = 0;
        let ctrl8bit = ((*vt).mode & VT_MODE_CTRL8BIT) != 0;

        if ctrl != 0 {
            if ctrl >= 0x80 && !ctrl8bit {
                if (*vt).tmpbuffer_len <= 2 {
                    return;
                }
                *(*vt).tmpbuffer = 0x1b as c_char;
                *(*vt).tmpbuffer.add(1) = (ctrl - 0x40) as c_char;
                cur = 2;
            } else {
                if (*vt).tmpbuffer_len <= 1 {
                    return;
                }
                *(*vt).tmpbuffer = ctrl as c_char;
                cur = 1;
            }
        }

        if cur + payload.len() >= (*vt).tmpbuffer_len {
            return;
        }

        ptr::copy_nonoverlapping(
            payload.as_ptr().cast::<c_char>(),
            (*vt).tmpbuffer.add(cur),
            payload.len(),
        );
        cur += payload.len();

        if term {
            let st: &[u8] = if ctrl8bit { b"\x9c" } else { b"\x1b\\" };
            if cur + st.len() >= (*vt).tmpbuffer_len {
                return;
            }
            ptr::copy_nonoverlapping(
                st.as_ptr().cast::<c_char>(),
                (*vt).tmpbuffer.add(cur),
                st.len(),
            );
            cur += st.len();
        }

        *(*vt).tmpbuffer.add(cur) = 0;
        vterm_push_output_bytes(vt, (*vt).tmpbuffer, cur);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn vterm_get_attr_type(attr: VTermAttr) -> VTermValueType {
    match attr {
        VTERM_ATTR_BOLD => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_UNDERLINE => VTERM_VALUETYPE_INT,
        VTERM_ATTR_ITALIC => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_BLINK => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_REVERSE => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_CONCEAL => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_STRIKE => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_FONT => VTERM_VALUETYPE_INT,
        VTERM_ATTR_FOREGROUND => VTERM_VALUETYPE_COLOR,
        VTERM_ATTR_BACKGROUND => VTERM_VALUETYPE_COLOR,
        VTERM_ATTR_SMALL => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_BASELINE => VTERM_VALUETYPE_INT,
        VTERM_ATTR_URI => VTERM_VALUETYPE_INT,
        VTERM_ATTR_DIM => VTERM_VALUETYPE_BOOL,
        VTERM_ATTR_OVERLINE => VTERM_VALUETYPE_BOOL,
        _ => 0,
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vterm_scroll_rect(
    mut rect: VTermRect,
    downward: c_int,
    rightward: c_int,
    moverect: Option<
        unsafe extern "C" fn(src: VTermRect, dest: VTermRect, user: *mut c_void) -> c_int,
    >,
    eraserect: Option<
        unsafe extern "C" fn(rect: VTermRect, selective: c_int, user: *mut c_void) -> c_int,
    >,
    user: *mut c_void,
) {
    unsafe {
        if downward.abs() >= rect.end_row - rect.start_row
            || rightward.abs() >= rect.end_col - rect.start_col
        {
            if let Some(erase_cb) = eraserect {
                let _ = erase_cb(rect, 0, user);
            }
            return;
        }

        let (src_start_col, src_end_col, dest_start_col, dest_end_col) = if rightward >= 0 {
            (
                rect.start_col + rightward,
                rect.end_col,
                rect.start_col,
                rect.end_col - rightward,
            )
        } else {
            let leftward = -rightward;
            (
                rect.start_col,
                rect.end_col - leftward,
                rect.start_col + leftward,
                rect.end_col,
            )
        };

        let (src_start_row, src_end_row, dest_start_row, dest_end_row) = if downward >= 0 {
            (
                rect.start_row + downward,
                rect.end_row,
                rect.start_row,
                rect.end_row - downward,
            )
        } else {
            let upward = -downward;
            (
                rect.start_row,
                rect.end_row - upward,
                rect.start_row + upward,
                rect.end_row,
            )
        };

        let src = VTermRect {
            start_row: src_start_row,
            end_row: src_end_row,
            start_col: src_start_col,
            end_col: src_end_col,
        };
        let dest = VTermRect {
            start_row: dest_start_row,
            end_row: dest_end_row,
            start_col: dest_start_col,
            end_col: dest_end_col,
        };

        if let Some(move_cb) = moverect {
            let _ = move_cb(dest, src, user);
        }

        if downward > 0 {
            rect.start_row = rect.end_row - downward;
        } else if downward < 0 {
            rect.end_row = rect.start_row - downward;
        }

        if rightward > 0 {
            rect.start_col = rect.end_col - rightward;
        } else if rightward < 0 {
            rect.end_col = rect.start_col - rightward;
        }

        if let Some(erase_cb) = eraserect {
            let _ = erase_cb(rect, 0, user);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vterm_get_attr_type_maps_attributes() {
        assert_eq!(vterm_get_attr_type(VTERM_ATTR_BOLD), VTERM_VALUETYPE_BOOL);
        assert_eq!(
            vterm_get_attr_type(VTERM_ATTR_UNDERLINE),
            VTERM_VALUETYPE_INT
        );
        assert_eq!(
            vterm_get_attr_type(VTERM_ATTR_FOREGROUND),
            VTERM_VALUETYPE_COLOR
        );
        assert_eq!(vterm_get_attr_type(VTERM_N_ATTRS), 0);
    }
}
