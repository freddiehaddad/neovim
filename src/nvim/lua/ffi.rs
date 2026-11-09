//! Raw `LuaJIT`, `luv`, and `tree-sitter` C FFI types and `extern "C"` declarations.

use std::ffi::{c_char, c_void};

pub type Boolean = bool;
pub type Float = f64;
pub type LuaRef = i32;
pub(crate) const LUA_NOREF: LuaRef = -2;
#[allow(non_camel_case_types)]
pub type lua_State = c_void;
#[allow(non_camel_case_types)]
pub type lua_CFunction = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State) -> i32>;
#[allow(non_camel_case_types)]
pub type lua_Number = f64;
#[allow(non_camel_case_types)]
pub type lua_Integer = isize;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct luaL_Reg {
    pub name: *const c_char,
    pub func: lua_CFunction,
}

unsafe impl Sync for luaL_Reg {}

#[repr(C)]
pub struct luaL_Buffer {
    pub p: *mut c_char,
    pub lvl: i32,
    pub lstate: *mut lua_State,
    pub buffer: [c_char; 8192],
}

impl luaL_Buffer {
    pub const fn new() -> Self {
        Self {
            p: std::ptr::null_mut(),
            lvl: 0,
            lstate: std::ptr::null_mut(),
            buffer: [0; 8192],
        }
    }
}

impl Default for luaL_Buffer {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
pub struct lua_Debug {
    pub event: i32,
    pub name: *const c_char,
    pub namewhat: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    pub currentline: i32,
    pub nups: i32,
    pub linedefined: i32,
    pub lastlinedefined: i32,
    pub short_src: [c_char; 60],
    pub i_ci: i32,
    pub _pad: [u8; 32],
}

impl lua_Debug {
    pub const fn new() -> Self {
        Self {
            event: 0,
            name: std::ptr::null(),
            namewhat: std::ptr::null(),
            what: std::ptr::null(),
            source: std::ptr::null(),
            currentline: 0,
            nups: 0,
            linedefined: 0,
            lastlinedefined: 0,
            short_src: [0; 60],
            i_ci: 0,
            _pad: [0; 32],
        }
    }
}

impl Default for lua_Debug {
    fn default() -> Self {
        Self::new()
    }
}

impl lua_Debug {
    #[inline]
    pub fn is_c_what(&self) -> bool {
        !self.what.is_null() && unsafe { *self.what.cast::<u8>() == b'C' }
    }

    #[inline]
    pub fn file_source_cstr(&self) -> Option<&std::ffi::CStr> {
        if !self.source.is_null() && unsafe { *self.source.cast::<u8>() == b'@' } {
            Some(unsafe { std::ffi::CStr::from_ptr(self.source.wrapping_add(1)) })
        } else {
            None
        }
    }
}

pub const LUA_MULTRET: i32 = -1;
pub const LUA_REGISTRYINDEX: i32 = -10000;
pub const LUA_GLOBALSINDEX: i32 = -10002;
pub(crate) const LUA_MINSTACK: i32 = 20;

#[cfg(feature = "vendor-bit")]
pub(crate) const LUA_TNONE: i32 = -1;
pub(crate) const LUA_TNIL: i32 = 0;
pub(crate) const LUA_TBOOLEAN: i32 = 1;
pub(crate) const LUA_TLIGHTUSERDATA: i32 = 2;
pub(crate) const LUA_TNUMBER: i32 = 3;
pub(crate) const LUA_TSTRING: i32 = 4;
pub(crate) const LUA_TTABLE: i32 = 5;
pub(crate) const LUA_TFUNCTION: i32 = 6;
pub(crate) const LUA_TUSERDATA: i32 = 7;

pub(crate) const LUA_ERRRUN: i32 = 2;
pub(crate) const LUA_ERRMEM: i32 = 4;

pub(crate) const LUAL_BUFFERSIZE: usize = if cfg!(windows) { 512 } else { 8192 };

#[allow(non_camel_case_types)]
pub type luv_CFpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        nargs: i32,
        nresults: i32,
        flags: i32,
    ) -> i32,
>;
#[allow(non_camel_case_types)]
pub type luv_CFcpcall = Option<
    unsafe extern "C-unwind" fn(
        lstate: *mut lua_State,
        func: lua_CFunction,
        ud: *mut c_void,
        flags: i32,
    ) -> i32,
>;
#[allow(non_camel_case_types)]
pub type luv_acquire_vm = Option<unsafe extern "C-unwind" fn() -> *mut lua_State>;
#[allow(non_camel_case_types)]
pub type luv_release_vm = Option<unsafe extern "C-unwind" fn(lstate: *mut lua_State)>;

unsafe extern "C-unwind" {
    pub fn lua_gettop(lstate: *mut lua_State) -> i32;
    pub fn lua_settop(lstate: *mut lua_State, idx: i32);
    pub fn lua_pushvalue(lstate: *mut lua_State, idx: i32);
    pub fn lua_remove(lstate: *mut lua_State, idx: i32);
    pub fn lua_insert(lstate: *mut lua_State, idx: i32);
    pub fn lua_replace(lstate: *mut lua_State, idx: i32);
    pub fn lua_checkstack(lstate: *mut lua_State, sz: i32) -> i32;
    pub fn lua_type(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_typename(lstate: *mut lua_State, tp: i32) -> *const c_char;
    pub fn lua_equal(lstate: *mut lua_State, idx1: i32, idx2: i32) -> i32;
    pub fn lua_rawequal(lstate: *mut lua_State, idx1: i32, idx2: i32) -> i32;
    pub fn lua_tonumber(lstate: *mut lua_State, idx: i32) -> lua_Number;
    pub fn lua_toboolean(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_tolstring(lstate: *mut lua_State, idx: i32, len: *mut usize) -> *const c_char;
    pub fn lua_objlen(lstate: *mut lua_State, idx: i32) -> usize;
    pub fn lua_tocfunction(lstate: *mut lua_State, idx: i32) -> lua_CFunction;
    pub fn lua_topointer(lstate: *mut lua_State, idx: i32) -> *const c_void;
    pub fn lua_pushnil(lstate: *mut lua_State);
    pub fn lua_pushnumber(lstate: *mut lua_State, n: lua_Number);
    pub fn lua_pushinteger(lstate: *mut lua_State, n: lua_Integer);
    pub fn lua_pushlstring(lstate: *mut lua_State, s: *const c_char, l: usize);
    #[link_name = "lua_pushstring"]
    pub(crate) fn c_lua_pushstring(lstate: *mut lua_State, s: *const c_char);
    pub fn lua_pushfstring(lstate: *mut lua_State, fmt: *const c_char, ...) -> *const c_char;
    pub fn lua_pushboolean(lstate: *mut lua_State, b: i32);
    pub fn lua_pushlightuserdata(lstate: *mut lua_State, p: *mut c_void);
    pub fn lua_gettable(lstate: *mut lua_State, idx: i32);
    #[link_name = "lua_getfield"]
    pub(crate) fn c_lua_getfield(lstate: *mut lua_State, idx: i32, k: *const c_char);
    pub fn lua_settable(lstate: *mut lua_State, idx: i32);
    #[link_name = "lua_setfield"]
    pub(crate) fn c_lua_setfield(lstate: *mut lua_State, idx: i32, k: *const c_char);
    pub fn lua_rawget(lstate: *mut lua_State, idx: i32);
    pub fn lua_rawgeti(lstate: *mut lua_State, idx: i32, n: i32);
    pub fn lua_createtable(lstate: *mut lua_State, narr: i32, nrec: i32);
    pub fn lua_newuserdata(lstate: *mut lua_State, sz: usize) -> *mut c_void;
    pub fn lua_getmetatable(lstate: *mut lua_State, objindex: i32) -> i32;
    pub fn lua_getfenv(lstate: *mut lua_State, idx: i32);
    pub fn lua_rawset(lstate: *mut lua_State, idx: i32);
    pub fn lua_rawseti(lstate: *mut lua_State, idx: i32, n: i32);
    pub fn lua_setmetatable(lstate: *mut lua_State, objindex: i32) -> i32;
    pub fn lua_setfenv(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_call(lstate: *mut lua_State, nargs: i32, nresults: i32);
    pub fn lua_pcall(lstate: *mut lua_State, nargs: i32, nresults: i32, errfunc: i32) -> i32;
    pub fn lua_error(lstate: *mut lua_State) -> i32;
    pub fn lua_next(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_concat(lstate: *mut lua_State, n: i32);
    pub fn lua_pushcclosure(lstate: *mut lua_State, fn_: lua_CFunction, n: i32);
    pub fn lua_isnumber(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_isstring(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_iscfunction(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_isuserdata(lstate: *mut lua_State, idx: i32) -> i32;
    pub fn lua_tointeger(lstate: *mut lua_State, idx: i32) -> lua_Integer;
    pub fn lua_touserdata(lstate: *mut lua_State, idx: i32) -> *mut c_void;
    pub fn luaL_newstate() -> *mut lua_State;
    pub fn luaL_openlibs(lstate: *mut lua_State);
    pub fn lua_close(lstate: *mut lua_State);
    pub fn luaL_ref(lstate: *mut lua_State, t: i32) -> i32;
    pub fn luaL_unref(lstate: *mut lua_State, t: i32, ref_: i32);
    #[link_name = "luaL_newmetatable"]
    pub(crate) fn c_luaL_newmetatable(lstate: *mut lua_State, tname: *const c_char) -> i32;
    #[link_name = "luaL_getmetafield"]
    pub(crate) fn c_luaL_getmetafield(lstate: *mut lua_State, obj: i32, e: *const c_char) -> i32;
    #[link_name = "luaL_checkudata"]
    pub(crate) fn c_luaL_checkudata(
        lstate: *mut lua_State,
        ud: i32,
        tname: *const c_char,
    ) -> *mut c_void;
    #[link_name = "luaL_register"]
    pub(crate) fn c_luaL_register(
        lstate: *mut lua_State,
        libname: *const c_char,
        l: *const luaL_Reg,
    );
    #[link_name = "luaL_checkstack"]
    pub(crate) fn c_luaL_checkstack(lstate: *mut lua_State, sz: i32, msg: *const c_char);
    pub fn luaL_checktype(lstate: *mut lua_State, narg: i32, t: i32);
    pub fn luaL_checkany(lstate: *mut lua_State, narg: i32);
    pub fn luaL_checklstring(lstate: *mut lua_State, numArg: i32, l: *mut usize) -> *const c_char;
    pub fn luaL_checknumber(lstate: *mut lua_State, numArg: i32) -> lua_Number;
    pub fn luaL_checkinteger(lstate: *mut lua_State, numArg: i32) -> lua_Integer;
    pub fn luaL_optinteger(lstate: *mut lua_State, nArg: i32, def: lua_Integer) -> lua_Integer;
    #[cfg(feature = "vendor-bit")]
    #[link_name = "luaL_typerror"]
    pub(crate) fn c_luaL_typerror(lstate: *mut lua_State, narg: i32, tname: *const c_char) -> i32;
    #[link_name = "luaL_argerror"]
    pub(crate) fn c_luaL_argerror(
        lstate: *mut lua_State,
        numarg: i32,
        extramsg: *const c_char,
    ) -> i32;
    pub fn luaL_where(lstate: *mut lua_State, lvl: i32);
    pub fn luaL_error(lstate: *mut lua_State, fmt: *const c_char, ...) -> i32;
    #[link_name = "luaL_loadbuffer"]
    pub(crate) fn c_luaL_loadbuffer(
        lstate: *mut lua_State,
        buff: *const c_char,
        sz: usize,
        name: *const c_char,
    ) -> i32;
    pub fn luaL_buffinit(lstate: *mut lua_State, B: *mut luaL_Buffer);
    pub fn luaL_prepbuffer(B: *mut luaL_Buffer) -> *mut c_char;
    pub fn luaL_addlstring(B: *mut luaL_Buffer, s: *const c_char, l: usize);
    pub fn luaL_addvalue(B: *mut luaL_Buffer);
    pub fn luaL_pushresult(B: *mut luaL_Buffer);
    pub fn lua_getstack(lstate: *mut lua_State, level: i32, ar: *mut lua_Debug) -> i32;
    #[link_name = "lua_getinfo"]
    pub(crate) fn c_lua_getinfo(
        lstate: *mut lua_State,
        what: *const c_char,
        ar: *mut lua_Debug,
    ) -> i32;

    pub fn luaopen_luv(lstate: *mut lua_State) -> i32;
}

unsafe extern "C" {
    pub fn luv_set_loop(lstate: *mut lua_State, loop_: *mut c_void);
    pub fn luv_set_callback(lstate: *mut lua_State, pcall: luv_CFpcall);
    pub fn luv_set_thread(lstate: *mut lua_State, pcall: luv_CFpcall);
    pub fn luv_set_cthread(lstate: *mut lua_State, cpcall: luv_CFcpcall);
    pub fn luv_set_thread_cb(acquire: luv_acquire_vm, release: luv_release_vm);

    #[cfg(unix)]
    pub fn pthread_exit(retval: *mut c_void) -> !;
}

#[repr(C)]
pub struct OptKeySet {
    pub is_set_: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeySetLink {
    pub str: Option<&'static std::ffi::CStr>,
    pub ptr_off: usize,
    pub key_type: i32,
    pub opt_index: i32,
    pub is_hlgroup: bool,
}

impl KeySetLink {
    #[inline]
    pub fn mark_set<T>(&self, retval: &mut T) {
        if self.opt_index >= 0 {
            let ks = std::ptr::from_mut(retval).cast::<OptKeySet>();
            unsafe {
                (*ks).is_set_ |= 1u64 << (self.opt_index as u32);
            }
        }
    }

    #[inline]
    pub fn is_field_set<T>(&self, value: &T) -> bool {
        if self.opt_index >= 0 {
            let ks = std::ptr::from_ref(value).cast::<OptKeySet>();
            unsafe { ((*ks).is_set_ & (1u64 << (self.opt_index as u32))) != 0 }
        } else {
            true
        }
    }

    #[inline]
    pub fn field_mut<'a, T, F>(&self, retval: &'a mut T) -> &'a mut F {
        unsafe {
            &mut *std::ptr::from_mut(retval)
                .cast::<u8>()
                .wrapping_add(self.ptr_off)
                .cast::<F>()
        }
    }

    #[inline]
    pub fn field_copy<T, F: Copy>(&self, value: &T) -> F {
        unsafe {
            *std::ptr::from_ref(value)
                .cast::<u8>()
                .wrapping_add(self.ptr_off)
                .cast::<F>()
        }
    }
}

#[inline]
pub fn lua_push_bytes(lstate: *mut lua_State, bytes: &[u8]) {
    unsafe {
        lua_pushlstring(
            lstate,
            if bytes.is_empty() {
                c"".as_ptr()
            } else {
                bytes.as_ptr().cast::<c_char>()
            },
            bytes.len(),
        )
    };
}

#[inline]
pub fn lua_pushstring(lstate: *mut lua_State, s: &std::ffi::CStr) {
    unsafe { c_lua_pushstring(lstate, s.as_ptr()) }
}

#[inline]
pub fn lua_pushstring_opt(lstate: *mut lua_State, s: Option<&std::ffi::CStr>) {
    unsafe { c_lua_pushstring(lstate, s.map_or(std::ptr::null(), std::ffi::CStr::as_ptr)) }
}

#[inline]
pub fn lua_getfield(lstate: *mut lua_State, idx: i32, k: &std::ffi::CStr) {
    unsafe { c_lua_getfield(lstate, idx, k.as_ptr()) }
}

#[inline]
pub fn lua_setfield(lstate: *mut lua_State, idx: i32, k: &std::ffi::CStr) {
    unsafe { c_lua_setfield(lstate, idx, k.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_newmetatable(lstate: *mut lua_State, tname: &std::ffi::CStr) -> i32 {
    unsafe { c_luaL_newmetatable(lstate, tname.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_getmetafield(lstate: *mut lua_State, obj: i32, e: &std::ffi::CStr) -> i32 {
    unsafe { c_luaL_getmetafield(lstate, obj, e.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkudata(lstate: *mut lua_State, ud: i32, tname: &std::ffi::CStr) -> *mut c_void {
    unsafe { c_luaL_checkudata(lstate, ud, tname.as_ptr()) }
}

#[inline]
pub fn lua_createtable_safe(lstate: *mut lua_State, narr: i32, nrec: i32) {
    unsafe { lua_createtable(lstate, narr, nrec) };
}

#[inline]
pub fn lua_settable_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_settable(lstate, idx) };
}

#[inline]
pub fn lua_rawset_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_rawset(lstate, idx) };
}

#[inline]
pub fn lua_rawseti_safe(lstate: *mut lua_State, idx: i32, n: i32) {
    unsafe { lua_rawseti(lstate, idx, n) };
}

#[inline]
pub fn lua_rawgeti_safe(lstate: *mut lua_State, idx: i32, n: i32) {
    unsafe { lua_rawgeti(lstate, idx, n) };
}

#[inline]
pub fn lua_getfenv_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_getfenv(lstate, idx) };
}

#[inline]
pub fn lua_setfenv_safe(lstate: *mut lua_State, idx: i32) -> i32 {
    unsafe { lua_setfenv(lstate, idx) }
}

#[inline]
pub fn lua_tointeger_safe(lstate: *mut lua_State, idx: i32) -> lua_Integer {
    unsafe { lua_tointeger(lstate, idx) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checknumber_safe(lstate: *mut lua_State, numarg: i32) -> lua_Number {
    unsafe { luaL_checknumber(lstate, numarg) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_ref_safe(lstate: *mut lua_State, t: i32) -> i32 {
    unsafe { luaL_ref(lstate, t) }
}

#[inline]
pub fn lua_pushcclosure_safe(
    lstate: *mut lua_State,
    f: extern "C-unwind" fn(*mut lua_State) -> i32,
    n: i32,
) {
    unsafe { lua_pushcclosure(lstate, Some(f), n) };
}

#[inline]
pub fn lua_touserdata_typed<T>(lstate: *mut lua_State, idx: i32) -> &'static mut T {
    unsafe { &mut *lua_touserdata(lstate, idx).cast::<T>() }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkudata_opt_typed<T>(
    lstate: *mut lua_State,
    ud: i32,
    tname: &std::ffi::CStr,
) -> Option<&'static mut T> {
    let ptr = luaL_checkudata(lstate, ud, tname).cast::<T>();
    (!ptr.is_null()).then(|| unsafe { &mut *ptr })
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_str(lstate: *mut lua_State, msg: &str) -> i32 {
    let cstr = std::ffi::CString::new(msg).unwrap_or_default();
    luaL_error_cstr(lstate, &cstr)
}

#[inline]
pub fn lua_setmetatable_safe(lstate: *mut lua_State, objindex: i32) -> i32 {
    unsafe { lua_setmetatable(lstate, objindex) }
}

#[inline]
pub fn lua_getmetatable_safe(lstate: *mut lua_State, objindex: i32) -> bool {
    unsafe { lua_getmetatable(lstate, objindex) != 0 }
}

#[inline]
pub fn lua_rawequal_safe(lstate: *mut lua_State, idx1: i32, idx2: i32) -> bool {
    unsafe { lua_rawequal(lstate, idx1, idx2) != 0 }
}

#[inline]
pub fn lua_checkstack_safe(lstate: *mut lua_State, sz: i32) -> bool {
    unsafe { lua_checkstack(lstate, sz) != 0 }
}

#[inline]
pub fn lua_settop_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_settop(lstate, idx) };
}

#[inline]
pub fn lua_type_safe(lstate: *mut lua_State, idx: i32) -> i32 {
    unsafe { lua_type(lstate, idx) }
}

#[inline]
pub fn lua_isnumber_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_isnumber(lstate, idx) != 0 }
}

#[inline]
pub fn lua_isstring_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_isstring(lstate, idx) != 0 }
}

#[inline]
pub fn lua_toboolean_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_toboolean(lstate, idx) != 0 }
}

#[inline]
pub fn lua_tonumber_safe(lstate: *mut lua_State, idx: i32) -> lua_Number {
    unsafe { lua_tonumber(lstate, idx) }
}

#[inline]
pub fn lua_objlen_safe(lstate: *mut lua_State, idx: i32) -> usize {
    unsafe { lua_objlen(lstate, idx) }
}

#[inline]
pub fn lua_next_safe(lstate: *mut lua_State, idx: i32) -> i32 {
    unsafe { lua_next(lstate, idx) }
}

#[inline]
pub fn lua_pushnil_safe(lstate: *mut lua_State) {
    unsafe { lua_pushnil(lstate) };
}

#[inline]
pub fn lua_pushboolean_safe(lstate: *mut lua_State, b: bool) {
    unsafe { lua_pushboolean(lstate, i32::from(b)) };
}

#[inline]
pub fn lua_pushinteger_safe(lstate: *mut lua_State, n: lua_Integer) {
    unsafe { lua_pushinteger(lstate, n) };
}

#[inline]
pub fn lua_pushnumber_safe(lstate: *mut lua_State, n: lua_Number) {
    unsafe { lua_pushnumber(lstate, n) };
}

#[inline]
pub fn lua_pushvalue_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_pushvalue(lstate, idx) };
}

#[inline]
pub fn lua_pushcfunction_safe(
    lstate: *mut lua_State,
    f: extern "C-unwind" fn(*mut lua_State) -> i32,
) {
    unsafe { lua_pushcclosure(lstate, Some(f), 0) };
}

#[inline]
pub fn lua_register_funcs(
    lstate: *mut lua_State,
    funcs: &[(
        &'static std::ffi::CStr,
        extern "C-unwind" fn(*mut lua_State) -> i32,
    )],
) {
    for &(name, func) in funcs {
        lua_pushcfunction_safe(lstate, func);
        lua_setfield(lstate, -2, name);
    }
}

#[inline]
pub fn lua_concat_safe(lstate: *mut lua_State, n: i32) {
    unsafe { lua_concat(lstate, n) };
}

#[inline]
pub fn lua_pcall_safe(lstate: *mut lua_State, nargs: i32, nresults: i32, errfunc: i32) -> i32 {
    unsafe { lua_pcall(lstate, nargs, nresults, errfunc) }
}

#[inline]
pub fn lua_error_safe(lstate: *mut lua_State) -> i32 {
    unsafe { lua_error(lstate) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_where_safe(lstate: *mut lua_State, lvl: i32) {
    unsafe { luaL_where(lstate, lvl) };
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_cstr(lstate: *mut lua_State, msg: &std::ffi::CStr) -> i32 {
    unsafe { luaL_error(lstate, c"%s".as_ptr(), msg.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_cstr_arg(
    lstate: *mut lua_State,
    fmt: &std::ffi::CStr,
    arg: &std::ffi::CStr,
) -> i32 {
    unsafe { luaL_error(lstate, fmt.as_ptr(), arg.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkinteger_safe(lstate: *mut lua_State, numarg: i32) -> lua_Integer {
    unsafe { luaL_checkinteger(lstate, numarg) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkbytes(lstate: *mut lua_State, numarg: i32) -> &'static [u8] {
    let mut len = 0usize;
    let ptr = unsafe { luaL_checklstring(lstate, numarg, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkbytes_with_nul(
    lstate: *mut lua_State,
    numarg: i32,
) -> (&'static [u8], &'static [u8]) {
    let mut len = 0usize;
    let ptr = unsafe { luaL_checklstring(lstate, numarg, &mut len) };
    if ptr.is_null() {
        (&[], &[0])
    } else {
        let with_nul = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len + 1) };
        (&with_nul[..len], with_nul)
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_check_nvim_string(
    lstate: *mut lua_State,
    numarg: i32,
) -> crate::api::private::validate::NvimString {
    let mut size = 0usize;
    let ptr = unsafe { luaL_checklstring(lstate, numarg, &mut size) };
    crate::api::private::validate::NvimString {
        data: crate::globals::GraphPtr::new(ptr.cast_mut().cast()),
        size,
    }
}

#[inline]
pub fn lua_tolstring_bytes(lstate: *mut lua_State, idx: i32) -> &'static [u8] {
    let mut len = 0usize;
    let ptr = unsafe { lua_tolstring(lstate, idx, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }
    }
}

#[inline]
pub fn lua_tolstring_opt_cstr(lstate: *mut lua_State, idx: i32) -> Option<&'static std::ffi::CStr> {
    let ptr = unsafe { lua_tolstring(lstate, idx, std::ptr::null_mut()) };
    (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr) })
}

#[inline]
pub fn lua_newuserdata_typed<T>(lstate: *mut lua_State, val: T) -> &'static mut T {
    unsafe {
        let p = lua_newuserdata(lstate, size_of::<T>()).cast::<T>();
        p.write(val);
        &mut *p
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkudata_typed<T>(
    lstate: *mut lua_State,
    ud: i32,
    tname: &std::ffi::CStr,
) -> &'static mut T {
    unsafe { &mut *luaL_checkudata(lstate, ud, tname).cast::<T>() }
}

#[inline]
pub fn lua_tolstring_cstr(lstate: *mut lua_State, idx: i32) -> &'static std::ffi::CStr {
    let ptr = unsafe { lua_tolstring(lstate, idx, std::ptr::null_mut()) };
    if ptr.is_null() {
        c""
    } else {
        unsafe { std::ffi::CStr::from_ptr(ptr) }
    }
}

#[inline]
pub fn nlua_gettop(lstate: *mut lua_State) -> i32 {
    unsafe { lua_gettop(lstate) }
}

#[inline]
pub fn nlua_fast_api_disabled_error(lstate: *mut lua_State, fn_name: &std::ffi::CStr) -> i32 {
    unsafe {
        luaL_error(
            lstate,
            crate::errors::E_FAST_API_DISABLED.as_ptr(),
            fn_name.as_ptr(),
        )
    }
}

#[inline]
pub fn nlua_finish_api_call(
    lstate: *mut lua_State,
    arena: &mut crate::memory::Arena,
    err: &mut crate::api::private::validate::Error,
    err_param: Option<&'static std::ffi::CStr>,
    nret: i32,
) -> i32 {
    crate::memory::arena_mem_free(crate::memory::arena_finish(arena));
    if err.err_type != crate::api::private::validate::ErrorType::ErrorTypeNone {
        luaL_where_safe(lstate, 1);

        if let Some(err_param) = err_param {
            lua_pushstring(lstate, c"Invalid '");
            lua_pushstring(lstate, err_param);
            lua_pushstring(lstate, c"': ");
        }
        lua_pushstring_opt(lstate, err.msg_cstr());
        crate::api::private::helpers::api_clear_error(err);

        lua_concat_safe(lstate, if err_param.is_some() { 5 } else { 2 });

        return lua_error_safe(lstate);
    }
    if nret < 0 { nlua_gettop(lstate) } else { nret }
}

#[inline]
pub fn nlua_register_api_table(
    lstate: *mut lua_State,
    api_functions: &[(
        &'static std::ffi::CStr,
        extern "C-unwind" fn(*mut lua_State) -> i32,
    )],
) {
    lua_createtable_safe(lstate, 0, api_functions.len() as i32);
    for &(name, func) in api_functions {
        lua_pushcfunction_safe(lstate, func);
        lua_setfield(lstate, -2, name);
    }
    lua_setfield(lstate, -2, c"api");
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_register(lstate: *mut lua_State, libname: Option<&std::ffi::CStr>, l: &[luaL_Reg]) {
    unsafe {
        c_luaL_register(
            lstate,
            libname.map_or(std::ptr::null(), std::ffi::CStr::as_ptr),
            l.as_ptr(),
        )
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkstack(lstate: *mut lua_State, sz: i32, msg: &std::ffi::CStr) {
    unsafe { c_luaL_checkstack(lstate, sz, msg.as_ptr()) }
}

#[cfg(feature = "vendor-bit")]
#[inline]
#[allow(non_snake_case)]
pub fn luaL_typerror(lstate: *mut lua_State, narg: i32, tname: &std::ffi::CStr) -> i32 {
    unsafe { c_luaL_typerror(lstate, narg, tname.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_argerror(lstate: *mut lua_State, numarg: i32, extramsg: &std::ffi::CStr) -> i32 {
    unsafe { c_luaL_argerror(lstate, numarg, extramsg.as_ptr()) }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_loadbuffer(lstate: *mut lua_State, buff: &[u8], name: &std::ffi::CStr) -> i32 {
    unsafe {
        c_luaL_loadbuffer(
            lstate,
            if buff.is_empty() {
                c"".as_ptr()
            } else {
                buff.as_ptr().cast()
            },
            buff.len(),
            name.as_ptr(),
        )
    }
}

#[inline]
pub fn lua_getinfo(lstate: *mut lua_State, what: &std::ffi::CStr, ar: &mut lua_Debug) -> i32 {
    unsafe { c_lua_getinfo(lstate, what.as_ptr(), ar) }
}

#[inline]
pub const fn lua_upvalueindex(i: i32) -> i32 {
    LUA_GLOBALSINDEX - i
}

#[inline]
pub fn lua_pop(lstate: *mut lua_State, n: i32) {
    lua_settop_safe(lstate, -n - 1);
}

#[inline]
pub fn lua_newtable(lstate: *mut lua_State) {
    lua_createtable_safe(lstate, 0, 0);
}

#[inline]
pub fn lua_pushcfunction(lstate: *mut lua_State, f: lua_CFunction) {
    unsafe {
        lua_pushcclosure(lstate, f, 0);
    }
}

#[inline]
pub fn lua_setglobal(lstate: *mut lua_State, s: &std::ffi::CStr) {
    lua_setfield(lstate, LUA_GLOBALSINDEX, s);
}

#[inline]
pub fn lua_getglobal(lstate: *mut lua_State, s: &std::ffi::CStr) {
    lua_getfield(lstate, LUA_GLOBALSINDEX, s);
}

#[inline]
pub fn lua_tostring(lstate: *mut lua_State, i: i32) -> Option<&'static std::ffi::CStr> {
    lua_tolstring_opt_cstr(lstate, i)
}

#[inline]
pub fn lua_isfunction(lstate: *mut lua_State, n: i32) -> bool {
    lua_type_safe(lstate, n) == LUA_TFUNCTION
}

#[inline]
pub fn lua_istable(lstate: *mut lua_State, n: i32) -> bool {
    lua_type_safe(lstate, n) == LUA_TTABLE
}

#[inline]
pub fn lua_isnil(lstate: *mut lua_State, n: i32) -> bool {
    lua_type_safe(lstate, n) == LUA_TNIL
}

#[inline]
pub fn lua_isboolean(lstate: *mut lua_State, n: i32) -> bool {
    lua_type_safe(lstate, n) == LUA_TBOOLEAN
}

#[inline]
pub fn lua_isnoneornil(lstate: *mut lua_State, n: i32) -> bool {
    lua_type_safe(lstate, n) <= 0
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_typename(lstate: *mut lua_State, i: i32) -> &'static std::ffi::CStr {
    let ptr = unsafe { lua_typename(lstate, lua_type(lstate, i)) };
    if ptr.is_null() {
        c""
    } else {
        unsafe { std::ffi::CStr::from_ptr(ptr) }
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_getmetatable(lstate: *mut lua_State, n: &std::ffi::CStr) {
    lua_getfield(lstate, LUA_REGISTRYINDEX, n);
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_argcheck(lstate: *mut lua_State, cond: bool, numarg: i32, extramsg: &std::ffi::CStr) {
    if !cond {
        luaL_argerror(lstate, numarg, extramsg);
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkstring(lstate: *mut lua_State, n: i32) -> &'static std::ffi::CStr {
    let ptr = unsafe { luaL_checklstring(lstate, n, std::ptr::null_mut()) };
    if ptr.is_null() {
        c""
    } else {
        unsafe { std::ffi::CStr::from_ptr(ptr) }
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkint(lstate: *mut lua_State, n: i32) -> i32 {
    luaL_checkinteger_safe(lstate, n) as i32
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_newstate_safe() -> *mut lua_State {
    unsafe { luaL_newstate() }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_openlibs_safe(lstate: *mut lua_State) {
    unsafe { luaL_openlibs(lstate) };
}

#[inline]
pub fn lua_close_safe(lstate: *mut lua_State) {
    unsafe { lua_close(lstate) };
}

#[inline]
pub fn lua_remove_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_remove(lstate, idx) };
}

#[inline]
pub fn lua_insert_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_insert(lstate, idx) };
}

#[inline]
pub fn lua_replace_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_replace(lstate, idx) };
}

#[inline]
pub fn lua_call_safe(lstate: *mut lua_State, nargs: i32, nresults: i32) {
    unsafe { lua_call(lstate, nargs, nresults) };
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checktype_safe(lstate: *mut lua_State, narg: i32, t: i32) {
    unsafe { luaL_checktype(lstate, narg, t) };
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_unref_safe(lstate: *mut lua_State, t: i32, ref_: i32) {
    unsafe { luaL_unref(lstate, t, ref_) };
}

#[inline]
pub fn lua_newuserdata_empty(lstate: *mut lua_State) {
    let _ = unsafe { lua_newuserdata(lstate, 0) };
}

#[inline]
pub fn lua_tolstring_cstr_with_len(
    lstate: *mut lua_State,
    idx: i32,
) -> Option<(&'static std::ffi::CStr, usize)> {
    let mut len = 0usize;
    let ptr = unsafe { lua_tolstring(lstate, idx, &mut len) };
    (!ptr.is_null()).then(|| (unsafe { std::ffi::CStr::from_ptr(ptr) }, len))
}

#[inline]
pub fn lua_iscfunction_eq(
    lstate: *mut lua_State,
    idx: i32,
    expected: extern "C-unwind" fn(*mut lua_State) -> i32,
) -> bool {
    unsafe {
        lua_iscfunction(lstate, idx) != 0
            && lua_tocfunction(lstate, idx).is_some_and(|f| {
                std::ptr::fn_addr_eq(
                    f,
                    expected as unsafe extern "C-unwind" fn(*mut lua_State) -> i32,
                )
            })
    }
}

#[inline]
pub fn lua_getstack_safe(lstate: *mut lua_State, level: i32, ar: &mut lua_Debug) -> i32 {
    unsafe { lua_getstack(lstate, level, ar) }
}

unsafe extern "C-unwind" fn luv_thread_cfcpcall_trampoline(
    lstate: *mut lua_State,
    func: lua_CFunction,
    ud: *mut c_void,
    flags: i32,
) -> i32 {
    unsafe {
        lua_pushcfunction(lstate, func);
        lua_pushlightuserdata(lstate, ud);
        crate::lua::executor::nlua_luv_thread_cfpcall(lstate, 1, 0, flags)
    }
}

#[inline]
pub fn luv_init_vim(
    lstate: *mut lua_State,
    is_thread: bool,
    main_loop: &mut crate::event::r#loop::Loop,
    fast_cb: extern "C-unwind" fn(*mut lua_State, i32, i32, i32) -> i32,
    thread_cb: extern "C-unwind" fn(*mut lua_State, i32, i32, i32) -> i32,
    thread_cb_cfpcall: extern "C-unwind" fn(*mut lua_State, i32, i32, i32) -> i32,
) {
    unsafe {
        if is_thread {
            luv_set_callback(lstate, Some(thread_cb_cfpcall));
            luv_set_thread(lstate, Some(thread_cb));
            luv_set_cthread(lstate, Some(luv_thread_cfcpcall_trampoline));
        } else {
            luv_set_loop(lstate, std::ptr::from_mut(main_loop).cast());
            luv_set_callback(lstate, Some(fast_cb));
        }
        let _ = luaopen_luv(lstate);
    }
}

#[inline]
pub fn luv_set_thread_cb_safe(
    acquire: extern "C-unwind" fn() -> *mut lua_State,
    release: extern "C-unwind" fn(*mut lua_State),
) {
    unsafe {
        luv_set_thread_cb(Some(acquire), Some(release));
    }
}

pub fn nlua_exit_thread_on_oom(lstate: *mut lua_State) -> ! {
    unsafe {
        lua_close(lstate);
        #[cfg(windows)]
        {
            crate::os::os_win_console::ExitThread(0);
        }
        #[cfg(not(windows))]
        {
            pthread_exit(std::ptr::null_mut());
        }
    }
}

#[inline]
pub fn lua_gettable_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_gettable(lstate, idx) };
}

#[inline]
pub fn lua_rawget_safe(lstate: *mut lua_State, idx: i32) {
    unsafe { lua_rawget(lstate, idx) };
}

#[inline]
pub fn lua_isuserdata_safe(lstate: *mut lua_State, idx: i32) -> bool {
    unsafe { lua_isuserdata(lstate, idx) != 0 }
}

#[inline]
pub fn lua_touserdata_opt_typed<T>(lstate: *mut lua_State, idx: i32) -> Option<&'static mut T> {
    let ptr = unsafe { lua_touserdata(lstate, idx) }.cast::<T>();
    (!ptr.is_null()).then(|| unsafe { &mut *ptr })
}

#[inline]
pub fn lua_drop_userdata<T>(lstate: *mut lua_State, idx: i32) {
    let ptr = unsafe { lua_touserdata(lstate, idx) }.cast::<T>();
    if !ptr.is_null() {
        unsafe { std::ptr::drop_in_place(ptr) };
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_checkudata_drop<T>(lstate: *mut lua_State, ud: i32, tname: &std::ffi::CStr) {
    let ptr = luaL_checkudata(lstate, ud, tname).cast::<T>();
    if !ptr.is_null() {
        unsafe { std::ptr::drop_in_place(ptr) };
    }
}

#[inline]
pub fn lua_newuserdata_opt_typed<T>(lstate: *mut lua_State, val: T) -> Option<&'static mut T> {
    let ptr = unsafe { lua_newuserdata(lstate, size_of::<T>()) }.cast::<T>();
    (!ptr.is_null()).then(|| unsafe {
        ptr.write(val);
        &mut *ptr
    })
}

#[inline]
pub fn lua_newuserdata_size(lstate: *mut lua_State, sz: usize) {
    let _ = unsafe { lua_newuserdata(lstate, sz) };
}

#[inline]
pub fn lua_pushlightuserdata_masked<T>(lstate: *mut lua_State, r: &'static T) {
    let ptr = std::ptr::from_ref(r);
    #[cfg(target_pointer_width = "64")]
    let masked = ((ptr as usize) & ((1usize << 47) - 1)) as *mut c_void;
    #[cfg(not(target_pointer_width = "64"))]
    let masked = ptr.cast_mut().cast::<c_void>();
    unsafe { lua_pushlightuserdata(lstate, masked) };
}

#[inline]
pub fn lua_touserdata_eq_static<T>(lstate: *mut lua_State, idx: i32, r: &'static T) -> bool {
    unsafe { lua_touserdata(lstate, idx) == std::ptr::from_ref(r).cast_mut().cast() }
}

#[inline]
pub fn lua_setfuncs_with_upvalues(
    lstate: *mut lua_State,
    funcs: &[(
        &'static std::ffi::CStr,
        extern "C-unwind" fn(*mut lua_State) -> i32,
    )],
    nup: i32,
) {
    luaL_checkstack(lstate, nup, c"too many upvalues");
    for &(name, func) in funcs {
        for _ in 0..nup {
            lua_pushvalue_safe(lstate, -nup);
        }
        lua_pushcclosure_safe(lstate, func, nup);
        lua_setfield(lstate, -(nup + 2), name);
    }
    lua_pop(lstate, nup);
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_register_safe(
    lstate: *mut lua_State,
    libname: Option<&std::ffi::CStr>,
    funcs: &[(
        &'static std::ffi::CStr,
        extern "C-unwind" fn(*mut lua_State) -> i32,
    )],
) {
    let mut regs = Vec::with_capacity(funcs.len() + 1);
    for &(name, func) in funcs {
        regs.push(luaL_Reg {
            name: name.as_ptr(),
            func: Some(func),
        });
    }
    regs.push(luaL_Reg {
        name: std::ptr::null(),
        func: None,
    });
    luaL_register(lstate, libname, &regs);
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_buffinit_safe(lstate: *mut lua_State, b: &mut luaL_Buffer) {
    unsafe { luaL_buffinit(lstate, b) };
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_prepbuffer_slice(b: &mut luaL_Buffer) -> &'static mut [u8] {
    unsafe {
        std::slice::from_raw_parts_mut(
            luaL_prepbuffer(b).cast::<u8>(),
            super::converter::LUAL_BUFFERSIZE,
        )
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_addsize(b: &mut luaL_Buffer, n: usize) {
    b.p = b.p.wrapping_add(n);
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_pushresult_safe(b: &mut luaL_Buffer) {
    unsafe { luaL_pushresult(b) };
}

#[inline]
pub fn lua_pushfstring_topointer(lstate: *mut lua_State, fmt: &std::ffi::CStr, idx: i32) {
    unsafe {
        lua_pushfstring(lstate, fmt.as_ptr(), lua_topointer(lstate, idx));
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_two_cstrs(
    lstate: *mut lua_State,
    fmt: &std::ffi::CStr,
    a: &std::ffi::CStr,
    b: &std::ffi::CStr,
) -> ! {
    unsafe {
        luaL_error(lstate, fmt.as_ptr(), a.as_ptr(), b.as_ptr());
        unreachable!()
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_int(lstate: *mut lua_State, fmt: &std::ffi::CStr, a: i32) -> ! {
    unsafe {
        luaL_error(lstate, fmt.as_ptr(), a);
        unreachable!()
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_two_ints(lstate: *mut lua_State, fmt: &std::ffi::CStr, a: i32, b: i32) -> ! {
    unsafe {
        luaL_error(lstate, fmt.as_ptr(), a, b);
        unreachable!()
    }
}

#[inline]
#[allow(non_snake_case)]
pub fn luaL_error_two_cstrs_int(
    lstate: *mut lua_State,
    fmt: &std::ffi::CStr,
    a: &std::ffi::CStr,
    b: &std::ffi::CStr,
    c: i32,
) -> ! {
    unsafe {
        luaL_error(lstate, fmt.as_ptr(), a.as_ptr(), b.as_ptr(), c);
        unreachable!()
    }
}

// Tree-sitter C API FFI types and declarations
pub type TSSymbol = u16;
pub type TSFieldId = u16;
pub type TSLanguage = c_void;
pub type TSParser = c_void;
pub type TSTree = c_void;
pub type TSQuery = c_void;
pub type TSQueryCursor = c_void;

pub type TSInputEncoding = i32;
pub type TSSymbolType = i32;
pub type TSLogType = i32;
pub type TSQueryPredicateStepType = i32;
pub type TSQueryError = i32;

pub type TSDecodeFunction =
    Option<unsafe extern "C" fn(string: *const u8, length: u32, code_point: *mut i32) -> u32>;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TSPoint {
    pub row: u32,
    pub column: u32,
}

impl TSPoint {
    pub const fn new() -> Self {
        Self { row: 0, column: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TSRange {
    pub start_point: TSPoint,
    pub end_point: TSPoint,
    pub start_byte: u32,
    pub end_byte: u32,
}

impl TSRange {
    pub const fn new() -> Self {
        Self {
            start_point: TSPoint::new(),
            end_point: TSPoint::new(),
            start_byte: 0,
            end_byte: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSInput {
    pub payload: *mut c_void,
    pub read: Option<
        unsafe extern "C" fn(
            payload: *mut c_void,
            byte_index: u32,
            position: TSPoint,
            bytes_read: *mut u32,
        ) -> *const u8,
    >,
    pub encoding: TSInputEncoding,
    pub decode: TSDecodeFunction,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSParseState {
    pub payload: *mut c_void,
    pub current_byte_offset: u32,
    pub has_error: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSParseOptions {
    pub payload: *mut c_void,
    pub progress_callback: Option<unsafe extern "C" fn(state: *mut TSParseState) -> bool>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSLogger {
    pub payload: *mut c_void,
    pub log:
        Option<unsafe extern "C" fn(payload: *mut c_void, log_type: TSLogType, buffer: *const u8)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSInputEdit {
    pub start_byte: u32,
    pub old_end_byte: u32,
    pub new_end_byte: u32,
    pub start_point: TSPoint,
    pub old_end_point: TSPoint,
    pub new_end_point: TSPoint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSNode {
    pub context: [u32; 4],
    pub id: *const c_void,
    pub tree: *const TSTree,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSQueryCapture {
    pub node: TSNode,
    pub index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSQueryMatch {
    pub id: u32,
    pub pattern_index: u16,
    pub capture_count: u16,
    pub captures: *const TSQueryCapture,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSQueryPredicateStep {
    pub type_: TSQueryPredicateStepType,
    pub value_id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TSLanguageMetadata {
    pub major_version: u8,
    pub minor_version: u8,
    pub patch_version: u8,
}

unsafe extern "C-unwind" {
    pub fn ts_parser_parse(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        input: TSInput,
    ) -> *mut TSTree;
    pub fn ts_parser_parse_with_options(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        input: TSInput,
        parse_options: TSParseOptions,
    ) -> *mut TSTree;
}

unsafe extern "C" {
    pub fn ts_parser_new() -> *mut TSParser;
    pub fn ts_parser_delete(self_: *mut TSParser);
    pub fn ts_parser_language(self_: *const TSParser) -> *const TSLanguage;
    pub fn ts_parser_set_language(self_: *mut TSParser, language: *const TSLanguage) -> bool;
    pub fn ts_parser_set_included_ranges(
        self_: *mut TSParser,
        ranges: *const TSRange,
        count: u32,
    ) -> bool;
    pub fn ts_parser_included_ranges(self_: *const TSParser, count: &mut u32) -> *const TSRange;
    pub fn ts_parser_parse_string(
        self_: *mut TSParser,
        old_tree: *const TSTree,
        string: *const u8,
        length: u32,
    ) -> *mut TSTree;
    pub fn ts_parser_reset(self_: *mut TSParser);
    pub fn ts_parser_set_logger(self_: *mut TSParser, logger: TSLogger);
    pub fn ts_parser_logger(self_: *const TSParser) -> TSLogger;

    pub fn ts_tree_copy(self_: *const TSTree) -> *mut TSTree;
    pub fn ts_tree_delete(self_: *mut TSTree);
    pub fn ts_tree_root_node(self_: *const TSTree) -> TSNode;
    pub fn ts_tree_included_ranges(self_: *const TSTree, length: &mut u32) -> *mut TSRange;
    pub fn ts_tree_edit(self_: *mut TSTree, edit: &TSInputEdit);
    pub fn ts_tree_get_changed_ranges(
        old_tree: *const TSTree,
        new_tree: *const TSTree,
        length: &mut u32,
    ) -> *mut TSRange;

    pub fn ts_node_type(self_: TSNode) -> *const u8;
    pub fn ts_node_symbol(self_: TSNode) -> TSSymbol;
    pub fn ts_node_start_byte(self_: TSNode) -> u32;
    pub fn ts_node_start_point(self_: TSNode) -> TSPoint;
    pub fn ts_node_end_byte(self_: TSNode) -> u32;
    pub fn ts_node_end_point(self_: TSNode) -> TSPoint;
    pub fn ts_node_string(self_: TSNode) -> *mut u8;
    pub fn ts_node_is_null(self_: TSNode) -> bool;
    pub fn ts_node_is_named(self_: TSNode) -> bool;
    pub fn ts_node_is_missing(self_: TSNode) -> bool;
    pub fn ts_node_is_extra(self_: TSNode) -> bool;
    pub fn ts_node_has_changes(self_: TSNode) -> bool;
    pub fn ts_node_has_error(self_: TSNode) -> bool;
    pub fn ts_node_parent(self_: TSNode) -> TSNode;
    pub fn ts_node_child_with_descendant(self_: TSNode, descendant: TSNode) -> TSNode;
    pub fn ts_node_child(self_: TSNode, child_index: u32) -> TSNode;
    pub fn ts_node_field_name_for_child(self_: TSNode, child_index: u32) -> *const u8;
    pub fn ts_node_child_count(self_: TSNode) -> u32;
    pub fn ts_node_named_child(self_: TSNode, child_index: u32) -> TSNode;
    pub fn ts_node_named_child_count(self_: TSNode) -> u32;
    pub fn ts_node_next_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_prev_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_next_named_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_prev_named_sibling(self_: TSNode) -> TSNode;
    pub fn ts_node_descendant_for_point_range(
        self_: TSNode,
        start: TSPoint,
        end: TSPoint,
    ) -> TSNode;
    pub fn ts_node_named_descendant_for_point_range(
        self_: TSNode,
        start: TSPoint,
        end: TSPoint,
    ) -> TSNode;
    pub fn ts_node_eq(self_: TSNode, other: TSNode) -> bool;

    pub fn ts_query_new(
        language: *const TSLanguage,
        source: *const u8,
        source_len: u32,
        error_offset: &mut u32,
        error_type: &mut TSQueryError,
    ) -> *mut TSQuery;
    pub fn ts_query_delete(self_: *mut TSQuery);
    pub fn ts_query_pattern_count(self_: *const TSQuery) -> u32;
    pub fn ts_query_capture_count(self_: *const TSQuery) -> u32;
    pub fn ts_query_predicates_for_pattern(
        self_: *const TSQuery,
        pattern_index: u32,
        step_count: &mut u32,
    ) -> *const TSQueryPredicateStep;
    pub fn ts_query_capture_name_for_id(
        self_: *const TSQuery,
        index: u32,
        length: &mut u32,
    ) -> *const u8;
    pub fn ts_query_string_value_for_id(
        self_: *const TSQuery,
        index: u32,
        length: &mut u32,
    ) -> *const u8;
    pub fn ts_query_disable_capture(self_: *mut TSQuery, name: *const u8, length: u32);
    pub fn ts_query_disable_pattern(self_: *mut TSQuery, pattern_index: u32);

    pub fn ts_query_cursor_new() -> *mut TSQueryCursor;
    pub fn ts_query_cursor_delete(self_: *mut TSQueryCursor);
    pub fn ts_query_cursor_exec(self_: *mut TSQueryCursor, query: *const TSQuery, node: TSNode);
    pub fn ts_query_cursor_set_match_limit(self_: *mut TSQueryCursor, limit: u32);
    pub fn ts_query_cursor_set_point_range(
        self_: *mut TSQueryCursor,
        start_point: TSPoint,
        end_point: TSPoint,
    ) -> bool;
    pub fn ts_query_cursor_next_match(self_: *mut TSQueryCursor, match_: &mut TSQueryMatch)
    -> bool;
    pub fn ts_query_cursor_remove_match(self_: *mut TSQueryCursor, match_id: u32);
    pub fn ts_query_cursor_next_capture(
        self_: *mut TSQueryCursor,
        match_: &mut TSQueryMatch,
        capture_index: &mut u32,
    ) -> bool;
    pub fn ts_query_cursor_set_max_start_depth(self_: *mut TSQueryCursor, max_start_depth: u32);

    pub fn ts_language_symbol_count(self_: *const TSLanguage) -> u32;
    pub fn ts_language_state_count(self_: *const TSLanguage) -> u32;
    pub fn ts_language_field_count(self_: *const TSLanguage) -> u32;
    pub fn ts_language_field_name_for_id(self_: *const TSLanguage, id: TSFieldId) -> *const u8;
    pub fn ts_language_supertypes(self_: *const TSLanguage, length: &mut u32) -> *const TSSymbol;
    pub fn ts_language_subtypes(
        self_: *const TSLanguage,
        supertype: TSSymbol,
        length: &mut u32,
    ) -> *const TSSymbol;
    pub fn ts_language_symbol_name(self_: *const TSLanguage, symbol: TSSymbol) -> *const u8;
    pub fn ts_language_symbol_type(self_: *const TSLanguage, symbol: TSSymbol) -> TSSymbolType;
    pub fn ts_language_abi_version(self_: *const TSLanguage) -> u32;
    pub fn ts_language_metadata(self_: *const TSLanguage) -> *const TSLanguageMetadata;
    pub fn ts_language_is_wasm(self_: *const TSLanguage) -> bool;

    pub fn ts_set_allocator(
        new_malloc: Option<unsafe extern "C" fn(usize) -> *mut c_void>,
        new_calloc: Option<unsafe extern "C" fn(usize, usize) -> *mut c_void>,
        new_realloc: Option<unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void>,
        new_free: Option<unsafe extern "C" fn(*mut c_void)>,
    );

    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_lua() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_c() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_diff() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_vim() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_markdown() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_markdown_inline() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_query() -> *const TSLanguage;
    #[cfg(target_os = "emscripten")]
    pub fn tree_sitter_vimdoc() -> *const TSLanguage;
}

pub struct TSOwnedRanges {
    ptr: *mut TSRange,
    len: usize,
}

impl TSOwnedRanges {
    #[inline]
    pub fn as_slice(&self) -> &[TSRange] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

impl Drop for TSOwnedRanges {
    fn drop(&mut self) {
        crate::memory::raw_dealloc(self.ptr.cast());
    }
}

pub struct TSOwnedCString {
    ptr: *mut u8,
}

impl TSOwnedCString {
    #[inline]
    pub fn as_cstr(&self) -> Option<&std::ffi::CStr> {
        (!self.ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(self.ptr.cast()) })
    }
}

impl Drop for TSOwnedCString {
    fn drop(&mut self) {
        crate::memory::raw_dealloc(self.ptr.cast());
    }
}

#[repr(C)]
struct TSLuaLoggerOpts {
    cb: LuaRef,
    lstate: *mut lua_State,
    lex: bool,
    parse: bool,
}

impl Drop for TSLuaLoggerOpts {
    fn drop(&mut self) {
        unsafe {
            luaL_unref(self.lstate, LUA_REGISTRYINDEX, self.cb);
        }
    }
}

unsafe extern "C" fn ts_malloc(size: usize) -> *mut c_void {
    crate::memory::raw_alloc(size)
}

unsafe extern "C" fn ts_calloc(count: usize, size: usize) -> *mut c_void {
    crate::memory::raw_calloc(count, size)
}

unsafe extern "C" fn ts_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    crate::memory::raw_realloc(ptr, size)
}

unsafe extern "C" fn ts_free(ptr: *mut c_void) {
    crate::memory::raw_dealloc(ptr)
}

#[inline]
pub fn ts_init_allocator() {
    unsafe {
        ts_set_allocator(
            Some(ts_malloc),
            Some(ts_calloc),
            Some(ts_realloc),
            Some(ts_free),
        );
    }
}

#[cfg(target_os = "emscripten")]
pub fn ts_get_static_parser(lang: Option<&[u8]>) -> *const TSLanguage {
    let Some(lang) = lang else {
        return std::ptr::null();
    };
    unsafe {
        match lang {
            b"lua" => tree_sitter_lua(),
            b"c" => tree_sitter_c(),
            b"diff" => tree_sitter_diff(),
            b"vim" => tree_sitter_vim(),
            b"markdown" => tree_sitter_markdown(),
            b"markdown_inline" => tree_sitter_markdown_inline(),
            b"query" => tree_sitter_query(),
            b"vimdoc" => tree_sitter_vimdoc(),
            _ => std::ptr::null(),
        }
    }
}

#[inline]
pub fn ts_lang_abi_version(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_abi_version(lang) }
}

#[inline]
pub fn ts_lang_symbol_count(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_symbol_count(lang) }
}

#[inline]
pub fn ts_lang_symbol_type(lang: *const TSLanguage, symbol: TSSymbol) -> TSSymbolType {
    unsafe { ts_language_symbol_type(lang, symbol) }
}

#[inline]
pub fn ts_lang_symbol_name(
    lang: *const TSLanguage,
    symbol: TSSymbol,
) -> Option<&'static std::ffi::CStr> {
    let ptr = unsafe { ts_language_symbol_name(lang, symbol) };
    (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
}

#[inline]
pub fn ts_lang_field_count(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_field_count(lang) }
}

#[inline]
pub fn ts_lang_field_name(
    lang: *const TSLanguage,
    id: TSFieldId,
) -> Option<&'static std::ffi::CStr> {
    let ptr = unsafe { ts_language_field_name_for_id(lang, id) };
    (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
}

#[inline]
pub fn ts_lang_is_wasm(lang: *const TSLanguage) -> bool {
    unsafe { ts_language_is_wasm(lang) }
}

#[inline]
pub fn ts_lang_metadata(lang: *const TSLanguage) -> Option<TSLanguageMetadata> {
    let ptr = unsafe { ts_language_metadata(lang) };
    (!ptr.is_null()).then(|| unsafe { *ptr })
}

#[inline]
pub fn ts_lang_state_count(lang: *const TSLanguage) -> u32 {
    unsafe { ts_language_state_count(lang) }
}

#[inline]
pub fn ts_lang_supertypes(lang: *const TSLanguage) -> &'static [TSSymbol] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_language_supertypes(lang, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_lang_subtypes_raw(
    lang: *const TSLanguage,
    supertype: TSSymbol,
) -> (*const TSSymbol, u32) {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_language_subtypes(lang, supertype, &mut len) };
    (ptr, len)
}

#[inline]
pub fn ts_lang_subtype_at(ptr: *const TSSymbol, idx: u32) -> TSSymbol {
    unsafe { *ptr.wrapping_add(idx as usize) }
}

#[inline]
pub fn ts_parser_create(lang: *const TSLanguage) -> Option<*mut TSParser> {
    unsafe {
        let p = ts_parser_new();
        if !ts_parser_set_language(p, lang) {
            ts_parser_delete(p);
            None
        } else {
            Some(p)
        }
    }
}

#[inline]
pub fn ts_parser_free(p: *mut TSParser) {
    unsafe {
        let logger = ts_parser_logger(p);
        if logger.log.is_some() {
            drop(Box::from_raw(logger.payload.cast::<TSLuaLoggerOpts>()));
        }
        ts_parser_delete(p);
    }
}

#[inline]
pub fn ts_parser_has_language(p: *const TSParser) -> bool {
    unsafe { !ts_parser_language(p).is_null() }
}

#[inline]
pub fn ts_parser_reset_safe(p: *mut TSParser) {
    unsafe {
        ts_parser_reset(p);
    }
}

#[inline]
pub fn ts_parser_parse_bytes(
    p: *mut TSParser,
    old_tree: *const TSTree,
    bytes: &[u8],
) -> *mut TSTree {
    unsafe {
        ts_parser_parse_string(
            p,
            old_tree,
            if bytes.is_empty() {
                c"".as_ptr().cast()
            } else {
                bytes.as_ptr()
            },
            bytes.len() as u32,
        )
    }
}

#[repr(C)]
struct TSLuaParserCallbackPayload {
    parse_start_time: u64,
    timeout_threshold_ns: u64,
}

unsafe extern "C" fn on_parser_progress(state: *mut TSParseState) -> bool {
    unsafe {
        let payload = (*state).payload.cast::<TSLuaParserCallbackPayload>();
        let parse_time = crate::os::time::os_hrtime().wrapping_sub((*payload).parse_start_time);
        parse_time >= (*payload).timeout_threshold_ns
    }
}

#[repr(C)]
struct TSInputCallbackContext {
    buf: *mut crate::buffer::FileBuf,
    read_cb: fn(&mut crate::buffer::FileBuf, TSPoint) -> (&'static [u8], u32),
}

unsafe extern "C" fn ts_input_read_trampoline(
    payload: *mut c_void,
    _byte_index: u32,
    position: TSPoint,
    bytes_read: *mut u32,
) -> *const u8 {
    unsafe {
        let ctx = &mut *payload.cast::<TSInputCallbackContext>();
        let (slice, count) = (ctx.read_cb)(&mut *ctx.buf, position);
        *bytes_read = count;
        if slice.is_empty() {
            c"".as_ptr().cast()
        } else {
            slice.as_ptr()
        }
    }
}

pub fn ts_parser_parse_with_input(
    p: *mut TSParser,
    old_tree: *const TSTree,
    buf: &mut crate::buffer::FileBuf,
    read_cb: fn(&mut crate::buffer::FileBuf, TSPoint) -> (&'static [u8], u32),
    timeout_ns: u64,
) -> *mut TSTree {
    unsafe {
        let mut ctx = TSInputCallbackContext {
            buf: std::ptr::from_mut(buf),
            read_cb,
        };
        let input = TSInput {
            payload: (&raw mut ctx).cast::<c_void>(),
            read: Some(ts_input_read_trampoline),
            encoding: 0,
            decode: None,
        };

        if timeout_ns == 0 {
            return ts_parser_parse(p, old_tree, input);
        }

        let mut payload = TSLuaParserCallbackPayload {
            parse_start_time: crate::os::time::os_hrtime(),
            timeout_threshold_ns: timeout_ns,
        };

        let parse_options = TSParseOptions {
            payload: (&raw mut payload).cast::<c_void>(),
            progress_callback: Some(on_parser_progress),
        };

        ts_parser_parse_with_options(p, old_tree, input, parse_options)
    }
}

#[inline]
pub fn ts_parser_set_ranges_slice(p: *mut TSParser, ranges: &[TSRange]) {
    unsafe {
        ts_parser_set_included_ranges(p, ranges.as_ptr(), ranges.len() as u32);
    }
}

#[inline]
pub fn ts_parser_get_ranges_slice(p: *const TSParser) -> &'static [TSRange] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_parser_included_ranges(p, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

unsafe extern "C" fn ts_logger_cb(payload: *mut c_void, logtype: TSLogType, s: *const u8) {
    const TSLOG_TYPE_PARSE: TSLogType = 0;
    const TSLOG_TYPE_LEX: TSLogType = 1;
    unsafe {
        let opts = &mut *payload.cast::<TSLuaLoggerOpts>();
        if (!opts.lex && logtype == TSLOG_TYPE_LEX) || (!opts.parse && logtype == TSLOG_TYPE_PARSE)
        {
            return;
        }

        let lstate = opts.lstate;
        lua_rawgeti(lstate, LUA_REGISTRYINDEX, opts.cb);
        crate::lua::converter::lua_pushstring(
            lstate,
            if logtype == TSLOG_TYPE_PARSE {
                c"parse"
            } else {
                c"lex"
            },
        );
        crate::lua::converter::lua_pushstring_opt(
            lstate,
            (!s.is_null()).then(|| std::ffi::CStr::from_ptr(s.cast())),
        );
        if lua_pcall(lstate, 2, 0, 0) != 0 {
            luaL_error(lstate, c"treesitter logger callback failed".as_ptr());
        }
    }
}

#[inline]
pub fn ts_parser_set_lua_logger(
    p: *mut TSParser,
    lstate: *mut lua_State,
    lex: bool,
    parse: bool,
    cb: LuaRef,
) {
    let opts = Box::into_raw(Box::new(TSLuaLoggerOpts {
        cb,
        lstate,
        lex,
        parse,
    }));
    let logger = TSLogger {
        payload: opts.cast::<c_void>(),
        log: Some(ts_logger_cb),
    };
    unsafe {
        ts_parser_set_logger(p, logger);
    }
}

#[inline]
pub fn ts_parser_get_lua_logger_cb(p: *const TSParser) -> Option<LuaRef> {
    unsafe {
        let logger = ts_parser_logger(p);
        logger.log.map(|_| {
            let opts = logger.payload.cast::<TSLuaLoggerOpts>();
            (*opts).cb
        })
    }
}

#[inline]
pub fn ts_tree_clone(tree: *const TSTree) -> *mut TSTree {
    unsafe { ts_tree_copy(tree) }
}

#[inline]
pub fn ts_tree_free(tree: *mut TSTree) {
    unsafe {
        ts_tree_delete(tree);
    }
}

#[inline]
pub fn ts_tree_root(tree: *const TSTree) -> TSNode {
    unsafe { ts_tree_root_node(tree) }
}

#[inline]
pub fn ts_tree_edit_safe(tree: *mut TSTree, edit: &TSInputEdit) {
    unsafe {
        ts_tree_edit(tree, edit);
    }
}

#[inline]
pub fn ts_tree_included_ranges_owned(tree: *const TSTree) -> TSOwnedRanges {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_tree_included_ranges(tree, &mut len) };
    TSOwnedRanges {
        ptr,
        len: len as usize,
    }
}

#[inline]
pub fn ts_tree_changed_ranges_owned(
    old_tree: *const TSTree,
    new_tree: *const TSTree,
) -> TSOwnedRanges {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_tree_get_changed_ranges(old_tree, new_tree, &mut len) };
    TSOwnedRanges {
        ptr,
        len: len as usize,
    }
}

impl TSNode {
    #[inline]
    pub const fn empty() -> Self {
        Self {
            context: [0; 4],
            id: std::ptr::null(),
            tree: std::ptr::null(),
        }
    }

    #[inline]
    pub fn is_null(self) -> bool {
        unsafe { ts_node_is_null(self) }
    }

    #[inline]
    pub fn type_cstr(self) -> Option<&'static std::ffi::CStr> {
        let ptr = unsafe { ts_node_type(self) };
        (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
    }

    #[inline]
    pub fn symbol(self) -> TSSymbol {
        unsafe { ts_node_symbol(self) }
    }

    #[inline]
    pub fn start_point(self) -> TSPoint {
        unsafe { ts_node_start_point(self) }
    }

    #[inline]
    pub fn end_point(self) -> TSPoint {
        unsafe { ts_node_end_point(self) }
    }

    #[inline]
    pub fn start_byte(self) -> u32 {
        unsafe { ts_node_start_byte(self) }
    }

    #[inline]
    pub fn end_byte(self) -> u32 {
        unsafe { ts_node_end_byte(self) }
    }

    #[inline]
    pub fn child_count(self) -> u32 {
        unsafe { ts_node_child_count(self) }
    }

    #[inline]
    pub fn named_child_count(self) -> u32 {
        unsafe { ts_node_named_child_count(self) }
    }

    #[inline]
    pub fn child(self, idx: u32) -> Self {
        unsafe { ts_node_child(self, idx) }
    }

    #[inline]
    pub fn named_child(self, idx: u32) -> Self {
        unsafe { ts_node_named_child(self, idx) }
    }

    #[inline]
    pub fn field_name_for_child(self, idx: u32) -> Option<&'static std::ffi::CStr> {
        let ptr = unsafe { ts_node_field_name_for_child(self, idx) };
        (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr.cast()) })
    }

    #[inline]
    pub fn is_named(self) -> bool {
        unsafe { ts_node_is_named(self) }
    }

    #[inline]
    pub fn is_missing(self) -> bool {
        unsafe { ts_node_is_missing(self) }
    }

    #[inline]
    pub fn is_extra(self) -> bool {
        unsafe { ts_node_is_extra(self) }
    }

    #[inline]
    pub fn has_changes(self) -> bool {
        unsafe { ts_node_has_changes(self) }
    }

    #[inline]
    pub fn has_error(self) -> bool {
        unsafe { ts_node_has_error(self) }
    }

    #[inline]
    pub fn sexpr_owned(self) -> TSOwnedCString {
        TSOwnedCString {
            ptr: unsafe { ts_node_string(self) },
        }
    }

    #[inline]
    pub fn descendant_for_point_range(self, start: TSPoint, end: TSPoint) -> Self {
        unsafe { ts_node_descendant_for_point_range(self, start, end) }
    }

    #[inline]
    pub fn named_descendant_for_point_range(self, start: TSPoint, end: TSPoint) -> Self {
        unsafe { ts_node_named_descendant_for_point_range(self, start, end) }
    }

    #[inline]
    pub fn parent(self) -> Self {
        unsafe { ts_node_parent(self) }
    }

    #[inline]
    pub fn child_with_descendant(self, descendant: Self) -> Self {
        unsafe { ts_node_child_with_descendant(self, descendant) }
    }

    #[inline]
    pub fn next_sibling(self) -> Self {
        unsafe { ts_node_next_sibling(self) }
    }

    #[inline]
    pub fn prev_sibling(self) -> Self {
        unsafe { ts_node_prev_sibling(self) }
    }

    #[inline]
    pub fn next_named_sibling(self) -> Self {
        unsafe { ts_node_next_named_sibling(self) }
    }

    #[inline]
    pub fn prev_named_sibling(self) -> Self {
        unsafe { ts_node_prev_named_sibling(self) }
    }

    #[inline]
    pub fn eq_node(self, other: Self) -> bool {
        unsafe { ts_node_eq(self, other) }
    }

    #[inline]
    pub fn id_ne_bytes(self) -> [u8; size_of::<*const c_void>()] {
        (self.id as usize).to_ne_bytes()
    }
}

impl TSQueryMatch {
    #[inline]
    pub fn captures_slice(&self) -> &[TSQueryCapture] {
        let len = self.capture_count as usize;
        if self.captures.is_null() || len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.captures, len) }
        }
    }
}

#[inline]
pub fn ts_query_create(
    lang: *const TSLanguage,
    src: &[u8],
) -> Result<*mut TSQuery, (usize, TSQueryError)> {
    let mut error_offset: u32 = 0;
    let mut error_type: TSQueryError = 0;
    let q = unsafe {
        ts_query_new(
            lang,
            if src.is_empty() {
                c"".as_ptr().cast()
            } else {
                src.as_ptr()
            },
            src.len() as u32,
            &mut error_offset,
            &mut error_type,
        )
    };
    if q.is_null() {
        Err((error_offset as usize, error_type))
    } else {
        Ok(q)
    }
}

#[inline]
pub fn ts_query_free(q: *mut TSQuery) {
    unsafe {
        ts_query_delete(q);
    }
}

#[inline]
pub fn ts_query_pattern_count_safe(q: *const TSQuery) -> u32 {
    unsafe { ts_query_pattern_count(q) }
}

#[inline]
pub fn ts_query_predicates_slice(
    q: *const TSQuery,
    pattern_index: u32,
) -> &'static [TSQueryPredicateStep] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_query_predicates_for_pattern(q, pattern_index, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_query_string_bytes(q: *const TSQuery, id: u32) -> &'static [u8] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_query_string_value_for_id(q, id, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_query_capture_count_safe(q: *const TSQuery) -> u32 {
    unsafe { ts_query_capture_count(q) }
}

#[inline]
pub fn ts_query_capture_name_bytes(q: *const TSQuery, index: u32) -> &'static [u8] {
    let mut len: u32 = 0;
    let ptr = unsafe { ts_query_capture_name_for_id(q, index, &mut len) };
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    }
}

#[inline]
pub fn ts_query_disable_capture_bytes(q: *mut TSQuery, name: &[u8]) {
    unsafe {
        ts_query_disable_capture(
            q,
            if name.is_empty() {
                c"".as_ptr().cast()
            } else {
                name.as_ptr()
            },
            name.len() as u32,
        );
    }
}

#[inline]
pub fn ts_query_disable_pattern_safe(q: *mut TSQuery, pattern_index: u32) {
    unsafe {
        ts_query_disable_pattern(q, pattern_index);
    }
}

#[inline]
pub fn ts_query_cursor_create() -> *mut TSQueryCursor {
    unsafe { ts_query_cursor_new() }
}

#[inline]
pub fn ts_query_cursor_free(c: *mut TSQueryCursor) {
    unsafe {
        ts_query_cursor_delete(c);
    }
}

#[inline]
pub fn ts_query_cursor_configure_and_exec(
    c: *mut TSQueryCursor,
    q: *const TSQuery,
    node: TSNode,
    start: TSPoint,
    end: TSPoint,
    max_start_depth: Option<u32>,
    match_limit: Option<u32>,
) {
    unsafe {
        ts_query_cursor_set_point_range(c, start, end);
        if let Some(depth) = max_start_depth {
            ts_query_cursor_set_max_start_depth(c, depth);
        }
        if let Some(limit) = match_limit {
            ts_query_cursor_set_match_limit(c, limit);
        }
        ts_query_cursor_exec(c, q, node);
    }
}

#[inline]
pub fn ts_query_cursor_remove_match_safe(c: *mut TSQueryCursor, match_id: u32) {
    unsafe {
        ts_query_cursor_remove_match(c, match_id);
    }
}

#[inline]
pub fn ts_query_cursor_next_capture_safe(
    c: *mut TSQueryCursor,
) -> Option<(TSQueryMatch, TSQueryCapture)> {
    let mut match_ = TSQueryMatch {
        id: 0,
        pattern_index: 0,
        capture_count: 0,
        captures: std::ptr::null(),
    };
    let mut capture_index: u32 = 0;
    unsafe {
        if !ts_query_cursor_next_capture(c, &mut match_, &mut capture_index) {
            return None;
        }
        let capture = *match_.captures.wrapping_add(capture_index as usize);
        Some((match_, capture))
    }
}

#[inline]
pub fn ts_query_cursor_next_match_safe(c: *mut TSQueryCursor) -> Option<TSQueryMatch> {
    let mut match_ = TSQueryMatch {
        id: 0,
        pattern_index: 0,
        capture_count: 0,
        captures: std::ptr::null(),
    };
    unsafe { ts_query_cursor_next_match(c, &mut match_).then_some(match_) }
}
