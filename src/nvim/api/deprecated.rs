//! Deprecated API functions (`src/nvim/api/deprecated.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

use crate::api::autocmd::{OptionalKeys, TryState};
use crate::api::buffer::{nvim_buf_get_lines, nvim_buf_set_lines};
use crate::api::extmark::{nvim_buf_clear_namespace, nvim_create_namespace, parse_virt_text};
use crate::api::options::{OptIndex, OptScope};
use crate::api::private::converter::sctx_T;
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, Object, ObjectData, ObjectType, api_err_exp, api_err_invalid,
};
use crate::api::tabpage::{Buffer, Tabpage, Window, tabpage_T, win_T};
use crate::api::vimscript::{KeyDict_exec_opts, exec_impl};
use crate::eval::decode::dict_T;
use crate::eval::deprecated::buf_T;
use crate::kvec::KVec;
use crate::lua::converter::Boolean;
use crate::map_glyph_cache::NvimString;
use crate::marktree::{
    DECOR_HIGHLIGHT_INLINE_INIT, DECOR_PRIORITY_BASE, DecorExt, DecorInlineData, MT_FLAG_DECOR_HL,
};
use crate::memory::{Arena, xmalloc, xrealloc};
use crate::msgpack_rpc::packer::Integer;
use crate::msgpack_rpc::unpacker::MsgpackRpcRequestHandler;

pub type colnr_T = c_int;
pub type LuaRetMode = c_int;
pub type VirtTextPos = c_int;
pub type VirtLineOverflow = c_int;
pub type StringBuilder = KVec<c_char>;

const MAXLNUM: i64 = 0x7fffffff;
const MAXCOL: i64 = 0x7fffffff;
const DECOR_ID_INVALID: u32 = u32::MAX;
const LINE_BUFFER_MIN_SIZE: usize = 4096;
const NL: c_char = b'\n' as c_char;
const NUL: c_char = 0;

const OPT_GLOBAL: c_int = 0x01;
const OPT_LOCAL: c_int = 0x02;

const kOptScopeGlobal: OptScope = 0;
const kOptScopeWin: OptScope = 1;
const kOptScopeBuf: OptScope = 2;

const kOptInvalid: OptIndex = -1;

const kRetObject: LuaRetMode = 0;
const kHlModeUnknown: u8 = 0;
const kVPosEndOfLine: VirtTextPos = 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorInline {
    pub ext: bool,
    pub data: DecorInlineData,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtTextChunk {
    pub text: *mut c_char,
    pub hl_id: c_int,
}

pub type VirtText = KVec<VirtTextChunk>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtLine {
    pub line: VirtText,
    pub flags: c_int,
    pub overflow: VirtLineOverflow,
}

pub type VirtLines = KVec<VirtLine>;

#[repr(C)]
#[derive(Clone, Copy)]
pub union DecorVirtTextData {
    pub virt_text: VirtText,
    pub virt_lines: VirtLines,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DecorVirtText {
    pub flags: u8,
    pub hl_mode: u8,
    pub priority: u16,
    pub width: c_int,
    pub col: c_int,
    pub pos: VirtTextPos,
    pub data: DecorVirtTextData,
    pub next: *mut DecorVirtText,
}

#[repr(C)]
pub struct KeyDict_empty {
    pub is_set__empty_: OptionalKeys,
}

pub type ApiDispatchWrapper = unsafe extern "C" fn(u64, Array, *mut Arena, *mut Error) -> Object;

unsafe extern "C" {
    static mut curbuf: *mut buf_T;
    static mut curwin: *mut win_T;
    static mut current_sctx: sctx_T;
    static mut msg_silent: c_int;
    static mut msg_didout: bool;
    static mut no_wait_return: c_int;
    static mut got_int: bool;

    fn nlua_exec(
        str: NvimString,
        name: *const c_char,
        args: Array,
        mode: LuaRetMode,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn find_buffer_by_handle(buffer: Buffer, err: *mut Error) -> *mut buf_T;
    fn find_window_by_handle(window: Window, err: *mut Error) -> *mut win_T;
    fn find_tab_by_handle(tabpage: Tabpage, err: *mut Error) -> *mut tabpage_T;
    fn syn_check_group(pp: *const c_char, len: usize) -> c_int;
    fn extmark_set(
        buf: *mut buf_T,
        ns_id: u32,
        idp: *mut u32,
        row: c_int,
        col: colnr_T,
        end_row: c_int,
        end_col: colnr_T,
        decor: DecorInline,
        decor_flags: u16,
        right_gravity: bool,
        end_right_gravity: bool,
        no_undo: bool,
        invalidate: bool,
        err: *mut Error,
    );
    fn decor_find_virttext(buf: *mut buf_T, row: c_int, ns_id: u32) -> *mut DecorVirtText;
    fn clear_virttext(text: *mut VirtText);
    fn syn_get_final_id(hl_id: c_int) -> c_int;
    fn syn_id2attr(hl_id: c_int) -> c_int;
    fn hl_get_attr_by_id(attr_id: c_int, rgb: Boolean, arena: *mut Arena, err: *mut Error) -> Dict;
    fn syn_name2id(name: *const c_char) -> c_int;
    fn dict_set_var(
        dict: *mut dict_T,
        name: NvimString,
        value: Object,
        del: bool,
        retval: bool,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Object;
    fn get_globvar_dict() -> *mut dict_T;
    fn get_option_info(
        name: NvimString,
        opt_flags: c_int,
        buf: *mut buf_T,
        win: *mut win_T,
        arena: *mut Arena,
        err: *mut Error,
    ) -> Dict;
    fn find_option(name: *const c_char) -> OptIndex;
    fn option_has_scope(opt_idx: OptIndex, scope: OptScope) -> bool;
    fn get_option_value_for(
        opt_idx: OptIndex,
        opt_flags: c_int,
        scope: OptScope,
        from: *mut c_void,
        err: *mut Error,
    ) -> Object;
    fn set_option_value_for(
        opt_idx: OptIndex,
        value: Object,
        opt_flags: c_int,
        scope: OptScope,
        from: *mut c_void,
        value_replaced: bool,
        err: *mut Error,
    );
    fn api_typename(t: ObjectType) -> *const c_char;
    fn api_set_error(err: *mut Error, err_type: ErrorType, format: *const c_char, ...);
    fn api_set_sctx(channel_id: u64) -> sctx_T;
    fn api_clear_error(value: *mut Error);
    fn api_free_object(value: Object);
    fn arena_array(arena: *mut Arena, max_size: usize) -> Array;
    fn copy_object(obj: Object, arena: *mut Arena) -> Object;
    fn copy_string(str: NvimString, arena: *mut Arena) -> NvimString;
    fn msgpack_rpc_get_handler_for(
        name: *const c_char,
        name_len: usize,
        error: *mut Error,
    ) -> MsgpackRpcRequestHandler;
    fn msg_ext_no_fast();
    fn emsg(s: *const c_char) -> bool;
    fn msg(s: *const c_char, attr: c_int) -> bool;
    fn msg_end();
    fn try_enter(tstate: *mut TryState);
    fn try_leave(tstate: *const TryState, err: *mut Error);
    fn win_setheight_win(height: c_int, win: *mut win_T, from_top: bool);
    fn win_setwidth_win(width: c_int, wp: *mut win_T, from_left: bool);
}

static mut OUT_LINE_BUF: StringBuilder = StringBuilder::new();
static mut ERR_LINE_BUF: StringBuilder = StringBuilder::new();

const NIL: Object = Object {
    obj_type: ObjectType::kObjectTypeNil,
    data: ObjectData { integer: 0 },
};

#[inline]
const fn integer_obj(i: Integer) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeInteger,
        data: ObjectData { integer: i },
    }
}

#[inline]
const fn string_obj(s: NvimString) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeString,
        data: ObjectData { string: s },
    }
}

#[inline]
const fn array_obj(a: Array) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeArray,
        data: ObjectData { array: a },
    }
}

#[inline]
const fn dict_obj(d: Dict) -> Object {
    Object {
        obj_type: ObjectType::kObjectTypeDict,
        data: ObjectData { dict: d },
    }
}

#[inline]
unsafe fn cstr_as_string(s: *const c_char) -> NvimString {
    if s.is_null() {
        NvimString {
            data: ptr::null_mut(),
            size: 0,
        }
    } else {
        let len = unsafe { CStr::from_ptr(s).to_bytes().len() };
        NvimString {
            data: s as *mut c_char,
            size: len,
        }
    }
}

#[inline]
const fn static_cstr_as_string(s: &'static CStr) -> NvimString {
    let bytes = s.to_bytes();
    NvimString {
        data: bytes.as_ptr() as *mut c_char,
        size: bytes.len(),
    }
}

#[inline]
unsafe fn add_c(arr: &mut Array, item: Object) {
    unsafe {
        *arr.items.add(arr.size) = item;
        arr.size += 1;
    }
}

#[inline]
unsafe fn error_set(err: *const Error) -> bool {
    unsafe { (*err).err_type != ErrorType::kErrorTypeNone }
}

#[inline]
unsafe fn api_err_range(err: *mut Error, name: *const c_char) {
    unsafe {
        api_err_invalid(err, name, c"out of range".as_ptr(), 0, false);
    }
}

unsafe fn try_wrap<F: FnOnce()>(err: *mut Error, f: F) {
    unsafe {
        let mut tstate: TryState = std::mem::zeroed();
        try_enter(&mut tstate);
        f();
        try_leave(&tstate, err);
    }
}

unsafe fn with_script_context<F: FnOnce()>(channel_id: u64, f: F) {
    unsafe {
        let save_current_sctx = api_set_sctx(channel_id);
        f();
        current_sctx = save_current_sctx;
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_exec(
    channel_id: u64,
    src: NvimString,
    output: Boolean,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let mut opts = KeyDict_exec_opts { output };
        exec_impl(channel_id, src, &mut opts, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_command_output(
    channel_id: u64,
    command: NvimString,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let mut opts = KeyDict_exec_opts { output: true };
        exec_impl(channel_id, command, &mut opts, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_execute_lua(
    code: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { nlua_exec(code, ptr::null(), args, kRetObject, arena, err) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_number(buffer: Buffer, err: *mut Error) -> Integer {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return 0;
        }
        i64::from((*buf).b_fnum)
    }
}

unsafe fn src2ns(src_id: *mut Integer) -> u32 {
    unsafe {
        if *src_id == 0 {
            *src_id = nvim_create_namespace(NvimString {
                data: ptr::null_mut(),
                size: 0,
            });
        }
        if *src_id < 0 {
            return (1u32 << 31) - 1;
        }
        *src_id as u32
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_clear_highlight(
    buffer: Buffer,
    ns_id: Integer,
    line_start: Integer,
    line_end: Integer,
    err: *mut Error,
) {
    unsafe {
        nvim_buf_clear_namespace(buffer, ns_id, line_start, line_end, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_add_highlight(
    buffer: Buffer,
    mut ns_id: Integer,
    hl_group: NvimString,
    line: Integer,
    col_start: Integer,
    mut col_end: Integer,
    err: *mut Error,
) -> Integer {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return 0;
        }

        if !(0..MAXLNUM).contains(&line) {
            api_err_range(err, c"line number".as_ptr());
            return 0;
        }
        if !(0..=MAXCOL).contains(&col_start) {
            api_err_range(err, c"column".as_ptr());
            return 0;
        }

        if !(0..=MAXCOL).contains(&col_end) {
            col_end = MAXCOL;
        }

        let ns = src2ns(&mut ns_id);

        if line >= i64::from((*buf).b_ml.ml_line_count) {
            return ns_id;
        }

        let hl_id = if hl_group.size > 0 {
            syn_check_group(hl_group.data, hl_group.size)
        } else {
            return ns_id;
        };

        let mut end_line = line as c_int;
        if col_end == MAXCOL {
            col_end = 0;
            end_line += 1;
        }

        let mut decor = DecorInline {
            ext: false,
            data: DecorInlineData {
                hl: DECOR_HIGHLIGHT_INLINE_INIT,
            },
        };
        decor.data.hl.hl_id = hl_id;

        extmark_set(
            buf,
            ns,
            ptr::null_mut(),
            line as c_int,
            col_start as colnr_T,
            end_line,
            col_end as colnr_T,
            decor,
            MT_FLAG_DECOR_HL,
            true,
            false,
            false,
            false,
            ptr::null_mut(),
        );
        ns_id
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_virtual_text(
    buffer: Buffer,
    mut src_id: Integer,
    line: Integer,
    chunks: Array,
    _opts: *mut KeyDict_empty,
    err: *mut Error,
) -> Integer {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return 0;
        }

        if !(0..MAXLNUM).contains(&line) {
            api_set_error(
                err,
                ErrorType::kErrorTypeValidation,
                c"Line number outside range".as_ptr(),
            );
            return 0;
        }

        let ns_id = src2ns(&mut src_id);
        let mut width: c_int = 0;

        let virt_text = parse_virt_text(chunks, err, &mut width, false);
        if error_set(err) {
            return 0;
        }

        let existing = decor_find_virttext(buf, line as c_int, ns_id);
        if !existing.is_null() {
            clear_virttext(&raw mut (*existing).data.virt_text);
            (*existing).data.virt_text = virt_text;
            (*existing).width = width;
            return src_id;
        }

        let vt = xmalloc(std::mem::size_of::<DecorVirtText>()).cast::<DecorVirtText>();
        *vt = DecorVirtText {
            flags: 0,
            hl_mode: kHlModeUnknown,
            priority: DECOR_PRIORITY_BASE,
            width,
            col: 0,
            pos: kVPosEndOfLine,
            data: DecorVirtTextData { virt_text },
            next: ptr::null_mut(),
        };
        (*vt).priority = 0;

        let decor = DecorInline {
            ext: true,
            data: DecorInlineData {
                ext: DecorExt {
                    sh_idx: DECOR_ID_INVALID,
                    vt: vt.cast(),
                },
            },
        };

        extmark_set(
            buf,
            ns_id,
            ptr::null_mut(),
            line as c_int,
            0,
            -1,
            -1,
            decor,
            0,
            true,
            false,
            false,
            false,
            ptr::null_mut(),
        );
        src_id
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_hl_by_id(
    hl_id: Integer,
    rgb: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let dic = Dict::new();
        if syn_get_final_id(hl_id as c_int) == 0 {
            api_err_invalid(err, c"highlight id".as_ptr(), ptr::null(), hl_id, false);
            return dic;
        }
        let attrcode = syn_id2attr(hl_id as c_int);
        hl_get_attr_by_id(attrcode, rgb, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_hl_by_name(
    name: NvimString,
    rgb: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let result = Dict::new();
        let id = syn_name2id(name.data);
        if id == 0 {
            api_err_invalid(err, c"highlight name".as_ptr(), name.data, 0, true);
            return result;
        }
        nvim_get_hl_by_id(i64::from(id), rgb, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_insert(
    buffer: Buffer,
    lnum: Integer,
    lines: Array,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        nvim_buf_set_lines(0, buffer, lnum, lnum, true, lines, arena, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_get_line(
    buffer: Buffer,
    mut index: Integer,
    arena: *mut Arena,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let mut rv = NvimString {
            data: ptr::null_mut(),
            size: 0,
        };
        index = convert_index(index);
        let slice = nvim_buf_get_lines(
            0,
            buffer,
            index,
            index + 1,
            true,
            arena,
            ptr::null_mut(),
            err,
        );
        if !error_set(err) && slice.size > 0 {
            rv = (*slice.items.add(0)).data.string;
        }
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_set_line(
    buffer: Buffer,
    mut index: Integer,
    line: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        let mut l = string_obj(line);
        let array = Array {
            items: &mut l,
            size: 1,
            capacity: 0,
        };
        index = convert_index(index);
        nvim_buf_set_lines(0, buffer, index, index + 1, true, array, arena, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_del_line(
    buffer: Buffer,
    mut index: Integer,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        let array = Array::new();
        index = convert_index(index);
        nvim_buf_set_lines(0, buffer, index, index + 1, true, array, arena, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_get_line_slice(
    buffer: Buffer,
    mut start: Integer,
    mut end: Integer,
    include_start: Boolean,
    include_end: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        start = convert_index(start) + i64::from(!include_start);
        end = convert_index(end) + i64::from(include_end);
        nvim_buf_get_lines(0, buffer, start, end, false, arena, ptr::null_mut(), err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_set_line_slice(
    buffer: Buffer,
    mut start: Integer,
    mut end: Integer,
    include_start: Boolean,
    include_end: Boolean,
    replacement: Array,
    arena: *mut Arena,
    err: *mut Error,
) {
    unsafe {
        start = convert_index(start) + i64::from(!include_start);
        end = convert_index(end) + i64::from(include_end);
        nvim_buf_set_lines(0, buffer, start, end, false, replacement, arena, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_set_var(
    buffer: Buffer,
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return NIL;
        }
        dict_set_var((*buf).b_vars, name, value, false, true, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn buffer_del_var(
    buffer: Buffer,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return NIL;
        }
        dict_set_var((*buf).b_vars, name, NIL, true, true, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn window_set_var(
    window: Window,
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return NIL;
        }
        dict_set_var((*win).w_vars, name, value, false, true, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn window_del_var(
    window: Window,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return NIL;
        }
        dict_set_var((*win).w_vars, name, NIL, true, true, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tabpage_set_var(
    tabpage: Tabpage,
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return NIL;
        }
        dict_set_var((*tab).tp_vars, name, value, false, true, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn tabpage_del_var(
    tabpage: Tabpage,
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let tab = find_tab_by_handle(tabpage, err);
        if tab.is_null() {
            return NIL;
        }
        dict_set_var((*tab).tp_vars, name, NIL, true, true, arena, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_set_var(
    name: NvimString,
    value: Object,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { dict_set_var(get_globvar_dict(), name, value, false, true, arena, err) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn vim_del_var(
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { dict_set_var(get_globvar_dict(), name, NIL, true, true, arena, err) }
}

#[inline]
const fn convert_index(index: i64) -> i64 {
    if index < 0 { index - 1 } else { index }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_option_info(
    name: NvimString,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe { get_option_info(name, OPT_GLOBAL, curbuf.cast(), curwin, arena, err) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_set_option(
    channel_id: u64,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        set_option_to(
            channel_id,
            ptr::null_mut(),
            kOptScopeGlobal,
            name,
            value,
            err,
        );
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_get_option(name: NvimString, err: *mut Error) -> Object {
    unsafe { get_option_from(ptr::null_mut(), kOptScopeGlobal, name, err) }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_get_option(
    buffer: Buffer,
    name: NvimString,
    err: *mut Error,
) -> Object {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return NIL;
        }
        get_option_from(buf.cast(), kOptScopeBuf, name, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_buf_set_option(
    channel_id: u64,
    buffer: Buffer,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        let buf = find_buffer_by_handle(buffer, err);
        if buf.is_null() {
            return;
        }
        set_option_to(channel_id, buf.cast(), kOptScopeBuf, name, value, err);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_get_option(
    window: Window,
    name: NvimString,
    err: *mut Error,
) -> Object {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return NIL;
        }
        get_option_from(win.cast(), kOptScopeWin, name, err)
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_set_option(
    channel_id: u64,
    window: Window,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        let win = find_window_by_handle(window, err);
        if win.is_null() {
            return;
        }
        set_option_to(channel_id, win.cast(), kOptScopeWin, name, value, err);
    }
}

unsafe fn get_option_from(
    from: *mut c_void,
    scope: OptScope,
    name: NvimString,
    err: *mut Error,
) -> Object {
    unsafe {
        if name.size == 0 {
            api_err_invalid(err, c"option name".as_ptr(), c"<empty>".as_ptr(), 0, true);
            return NIL;
        }

        let opt_idx = find_option(name.data);
        if opt_idx == kOptInvalid {
            api_err_invalid(err, c"option name".as_ptr(), name.data, 0, true);
            return NIL;
        }

        if !option_has_scope(opt_idx, scope) {
            api_err_invalid(err, c"option name".as_ptr(), name.data, 0, true);
            return NIL;
        }

        let value = get_option_value_for(
            opt_idx,
            if scope == kOptScopeGlobal {
                OPT_GLOBAL
            } else {
                OPT_LOCAL
            },
            scope,
            from,
            err,
        );
        if error_set(err) {
            return NIL;
        }

        value
    }
}

unsafe fn set_option_to(
    channel_id: u64,
    to: *mut c_void,
    scope: OptScope,
    name: NvimString,
    value: Object,
    err: *mut Error,
) {
    unsafe {
        if name.size == 0 {
            api_err_invalid(err, c"option name".as_ptr(), c"<empty>".as_ptr(), 0, true);
            return;
        }

        let opt_idx = find_option(name.data);
        if opt_idx == kOptInvalid {
            api_err_invalid(err, c"option name".as_ptr(), name.data, 0, true);
            return;
        }

        let valid = matches!(
            value.obj_type,
            ObjectType::kObjectTypeNil
                | ObjectType::kObjectTypeBoolean
                | ObjectType::kObjectTypeInteger
                | ObjectType::kObjectTypeString
        );
        if !valid {
            api_err_exp(
                err,
                c"value".as_ptr(),
                c"valid option type".as_ptr(),
                api_typename(value.obj_type),
            );
            return;
        }

        let opt_flags = if scope == kOptScopeWin && !option_has_scope(opt_idx, kOptScopeGlobal) {
            0
        } else if scope == kOptScopeGlobal {
            OPT_GLOBAL
        } else {
            OPT_LOCAL
        };

        with_script_context(channel_id, || {
            set_option_value_for(opt_idx, value, opt_flags, scope, to, true, err);
        });
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_call_atomic(
    channel_id: u64,
    calls: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Array {
    unsafe {
        let mut rv = arena_array(arena, 2);
        let mut results = arena_array(arena, calls.size);
        let mut nested_error = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: ptr::null_mut(),
        };

        let mut i = 0usize;
        'theend: {
            while i < calls.size {
                let item = *calls.items.add(i);
                if item.obj_type != ObjectType::kObjectTypeArray {
                    api_err_exp(
                        err,
                        c"'calls' item".as_ptr(),
                        api_typename(ObjectType::kObjectTypeArray),
                        api_typename(item.obj_type),
                    );
                    break 'theend;
                }
                let call = item.data.array;
                if call.size != 2 {
                    api_err_exp(
                        err,
                        c"'calls' item".as_ptr(),
                        c"2-item Array".as_ptr(),
                        ptr::null(),
                    );
                    break 'theend;
                }
                let first = *call.items.add(0);
                if first.obj_type != ObjectType::kObjectTypeString {
                    api_err_exp(
                        err,
                        c"name".as_ptr(),
                        api_typename(ObjectType::kObjectTypeString),
                        api_typename(first.obj_type),
                    );
                    break 'theend;
                }
                let name = first.data.string;
                let second = *call.items.add(1);
                if second.obj_type != ObjectType::kObjectTypeArray {
                    api_err_exp(
                        err,
                        c"call args".as_ptr(),
                        api_typename(ObjectType::kObjectTypeArray),
                        api_typename(second.obj_type),
                    );
                    break 'theend;
                }
                let args = second.data.array;

                let handler = msgpack_rpc_get_handler_for(name.data, name.size, &mut nested_error);
                if error_set(&nested_error) {
                    break;
                }

                let func: ApiDispatchWrapper = std::mem::transmute(handler.fn_ptr);
                let result = func(channel_id, args, arena, &mut nested_error);
                if error_set(&nested_error) {
                    break;
                }
                add_c(&mut results, copy_object(result, arena));
                if handler.ret_alloc {
                    api_free_object(result);
                }

                i += 1;
            }

            add_c(&mut rv, array_obj(results));
            if error_set(&nested_error) {
                let mut errval = arena_array(arena, 3);
                add_c(&mut errval, integer_obj(i as Integer));
                add_c(&mut errval, integer_obj(nested_error.err_type as Integer));
                add_c(
                    &mut errval,
                    string_obj(copy_string(cstr_as_string(nested_error.msg), arena)),
                );
                add_c(&mut rv, array_obj(errval));
            } else {
                add_c(&mut rv, NIL);
            }
        }

        api_clear_error(&mut nested_error);
        rv
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_subscribe(_channel_id: u64, _event: NvimString) {}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_unsubscribe(_channel_id: u64, _event: NvimString) {}

unsafe fn push_char(line_buf: &mut StringBuilder, c: c_char, to_err: bool) {
    unsafe {
        if line_buf.capacity == 0 {
            line_buf.capacity = LINE_BUFFER_MIN_SIZE;
            line_buf.items = xrealloc(line_buf.items.cast(), LINE_BUFFER_MIN_SIZE).cast();
        }
        if c == NL {
            line_buf.push(NUL);
            if to_err {
                emsg(line_buf.items);
            } else {
                msg(line_buf.items, 0);
            }
            if msg_silent == 0 {
                msg_didout = true;
            }
            line_buf.size = 0;
            line_buf.capacity = LINE_BUFFER_MIN_SIZE;
            line_buf.items = xrealloc(line_buf.items.cast(), LINE_BUFFER_MIN_SIZE).cast();
        } else if c == NUL {
            line_buf.push(NL);
        } else {
            line_buf.push(c);
        }
    }
}

unsafe fn write_msg(message: NvimString, to_err: bool, writeln: bool) {
    unsafe {
        let line_buf_ptr = if to_err {
            &raw mut ERR_LINE_BUF
        } else {
            &raw mut OUT_LINE_BUF
        };
        let line_buf = &mut *line_buf_ptr;

        msg_ext_no_fast();
        no_wait_return += 1;
        for i in 0..message.size {
            if got_int {
                break;
            }
            push_char(line_buf, *message.data.add(i), to_err);
        }
        if writeln {
            push_char(line_buf, NL, to_err);
        }
        no_wait_return -= 1;
        msg_end();
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_out_write(str: NvimString) {
    unsafe {
        write_msg(str, false, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_err_write(str: NvimString) {
    unsafe {
        write_msg(str, true, false);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_err_writeln(str: NvimString) {
    unsafe {
        write_msg(str, true, true);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_notify(
    msg_str: NvimString,
    log_level: Integer,
    opts: Dict,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut items = [string_obj(msg_str), integer_obj(log_level), dict_obj(opts)];
        let args = Array {
            size: 3,
            capacity: 3,
            items: items.as_mut_ptr(),
        };
        nlua_exec(
            static_cstr_as_string(c"return vim.notify(...)"),
            ptr::null(),
            args,
            kRetObject,
            arena,
            err,
        )
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_set_height(win: Window, height: Integer, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        try_wrap(err, || {
            win_setheight_win(height as c_int, w, true);
        });
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn nvim_win_set_width(win: Window, width: Integer, err: *mut Error) {
    unsafe {
        let w = find_window_by_handle(win, err);
        if w.is_null() {
            return;
        }
        try_wrap(err, || {
            win_setwidth_win(width as c_int, w, true);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_index_shifts_negative_indices() {
        assert_eq!(convert_index(0), 0);
        assert_eq!(convert_index(5), 5);
        assert_eq!(convert_index(-1), -2);
        assert_eq!(convert_index(-3), -4);
    }
}
