//! Msgpack-RPC serializer (`src/nvim/msgpack_rpc/packer.c`).

#[allow(unused_imports)]
use crate::lua::executor::{api_free_luaref, nlua_funcref_str};
use std::ffi::{CStr, c_char, c_int, c_void};

use crate::api::private::validate::{Array, Object, ObjectData, ObjectType};
use crate::kvec::KVecWithInit;
use crate::map_glyph_cache::NvimString;
use crate::memory::{xfree, xmalloc, xrealloc};

pub const MPACK_ITEM_SIZE: usize = 9;
const EXT_OBJECT_TYPE_SHIFT: c_int = ObjectType::kObjectTypeBuffer as c_int;
const LUA_NOREF: c_int = -2;

#[allow(non_camel_case_types)]
pub type handle_T = c_int;
pub type Integer = i64;
pub type PackerBufferFlush = Option<unsafe extern "C" fn(buffer: *mut PackerBuffer)>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PackerBuffer {
    pub startptr: *mut c_char,
    pub ptr: *mut c_char,
    pub endptr: *mut c_char,
    pub anydata: *mut c_void,
    pub anyint: i64,
    pub packer_flush: PackerBufferFlush,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ContainerStackItem {
    container: *mut Object,
    idx: usize,
}

#[inline]
pub(crate) unsafe fn mpack_w(b: *mut *mut c_char, byte: u8) {
    unsafe {
        **b = byte as c_char;
        *b = (*b).add(1);
    }
}

#[inline]
unsafe fn mpack_w2(b: *mut *mut c_char, v: u32) {
    let bytes = (v as u16).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*b).cast::<u8>(), 2);
        *b = (*b).add(2);
    }
}

#[inline]
unsafe fn mpack_w4(b: *mut *mut c_char, v: u32) {
    let bytes = v.to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*b).cast::<u8>(), 4);
        *b = (*b).add(4);
    }
}

#[inline]
unsafe fn mpack_w8(b: *mut *mut c_char, v: u64) {
    let bytes = v.to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*b).cast::<u8>(), 8);
        *b = (*b).add(8);
    }
}

#[inline]
pub(crate) unsafe fn mpack_uint(buf: *mut *mut c_char, val: u32) {
    unsafe {
        if val > 0xffff {
            mpack_w(buf, 0xce);
            mpack_w4(buf, val);
        } else if val > 0xff {
            mpack_w(buf, 0xcd);
            mpack_w2(buf, val);
        } else if val > 0x7f {
            mpack_w(buf, 0xcc);
            mpack_w(buf, val as u8);
        } else {
            mpack_w(buf, val as u8);
        }
    }
}

#[inline]
pub(crate) unsafe fn mpack_nil(buf: *mut *mut c_char) {
    unsafe {
        mpack_w(buf, 0xc0);
    }
}

#[inline]
pub(crate) unsafe fn mpack_bool(buf: *mut *mut c_char, val: bool) {
    unsafe {
        mpack_w(buf, 0xc2 | u8::from(val));
    }
}

#[inline]
pub(crate) unsafe fn mpack_array(buf: *mut *mut c_char, len: u32) {
    unsafe {
        if len < 0x10 {
            mpack_w(buf, 0x90 | (len as u8));
        } else if len < 0x10000 {
            mpack_w(buf, 0xdc);
            mpack_w2(buf, len);
        } else {
            mpack_w(buf, 0xdd);
            mpack_w4(buf, len);
        }
    }
}

#[inline]
pub(crate) unsafe fn mpack_map(buf: *mut *mut c_char, len: u32) {
    unsafe {
        if len < 0x10 {
            mpack_w(buf, 0x80 | (len as u8));
        } else if len < 0x10000 {
            mpack_w(buf, 0xde);
            mpack_w2(buf, len);
        } else {
            mpack_w(buf, 0xdf);
            mpack_w4(buf, len);
        }
    }
}

#[inline]
fn mpack_remaining(packer: &PackerBuffer) -> usize {
    (packer.endptr as usize).saturating_sub(packer.ptr as usize)
}

/// Ensure at least `2 * MPACK_ITEM_SIZE` bytes remain in `packer` (`mpack_check_buffer`).
///
/// # Safety
/// `packer` must be a valid writable pointer to a `PackerBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_check_buffer(packer: *mut PackerBuffer) {
    let p = unsafe { &mut *packer };
    if mpack_remaining(p) < 2 * MPACK_ITEM_SIZE
        && let Some(flush) = p.packer_flush
    {
        unsafe {
            flush(packer);
        }
    }
}

/// Pack a 64-bit unsigned integer into `*ptr` (`mpack_uint64`).
///
/// # Safety
/// `ptr` and `*ptr` must be valid writable pointers with sufficient space.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_uint64(ptr: *mut *mut c_char, i: u64) {
    unsafe {
        if i > 0x0fff_ffff {
            mpack_w(ptr, 0xcf);
            mpack_w8(ptr, i);
        } else {
            mpack_uint(ptr, i as u32);
        }
    }
}

/// Pack a signed 64-bit integer into `*ptr` (`mpack_integer`).
///
/// # Safety
/// `ptr` and `*ptr` must be valid writable pointers with sufficient space.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_integer(ptr: *mut *mut c_char, i: Integer) {
    unsafe {
        if i >= 0 {
            mpack_uint64(ptr, i as u64);
        } else if i < -0x8000_0000 {
            mpack_w(ptr, 0xd3);
            mpack_w8(ptr, i as u64);
        } else if i < -0x8000 {
            mpack_w(ptr, 0xd2);
            mpack_w4(ptr, i as u32);
        } else if i < -0x80 {
            mpack_w(ptr, 0xd1);
            mpack_w2(ptr, i as u32);
        } else if i < -0x20 {
            mpack_w(ptr, 0xd0);
            mpack_w(ptr, i as u8);
        } else {
            mpack_w(ptr, i as u8);
        }
    }
}

/// Pack an IEEE 754 `f64` into `*ptr` (`mpack_float8`).
///
/// # Safety
/// `ptr` and `*ptr` must be valid writable pointers with sufficient space.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_float8(ptr: *mut *mut c_char, i: f64) {
    unsafe {
        mpack_w(ptr, 0xcb);
        mpack_w8(ptr, i.to_bits());
    }
}

/// Pack a msgpack string into `packer` (`mpack_str`).
///
/// # Safety
/// `str_val` and `packer` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_str(str_val: NvimString, packer: *mut PackerBuffer) {
    let len = str_val.size;
    let p = unsafe { &mut *packer };
    unsafe {
        if len < 32 {
            mpack_w(&mut p.ptr, 0xa0 | (len as u8));
        } else if len < 0xff {
            mpack_w(&mut p.ptr, 0xd9);
            mpack_w(&mut p.ptr, len as u8);
        } else if len < 0xffff {
            mpack_w(&mut p.ptr, 0xda);
            mpack_w2(&mut p.ptr, len as u32);
        } else if len < 0xffff_ffff {
            mpack_w(&mut p.ptr, 0xdb);
            mpack_w4(&mut p.ptr, len as u32);
        } else {
            std::process::abort();
        }
        mpack_raw(str_val.data, len, packer);
    }
}

/// Pack a msgpack binary blob into `packer` (`mpack_bin`).
///
/// # Safety
/// `str_val` and `packer` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_bin(str_val: NvimString, packer: *mut PackerBuffer) {
    let len = str_val.size;
    let p = unsafe { &mut *packer };
    unsafe {
        if len < 0xff {
            mpack_w(&mut p.ptr, 0xc4);
            mpack_w(&mut p.ptr, len as u8);
        } else if len < 0xffff {
            mpack_w(&mut p.ptr, 0xc5);
            mpack_w2(&mut p.ptr, len as u32);
        } else if len < 0xffff_ffff {
            mpack_w(&mut p.ptr, 0xc6);
            mpack_w4(&mut p.ptr, len as u32);
        } else {
            std::process::abort();
        }
        mpack_raw(str_val.data, len, packer);
    }
}

/// Write raw bytes `data[..len]` into `packer`, flushing as needed (`mpack_raw`).
///
/// # Safety
/// `data` must be valid for `len` bytes when `len > 0`, and `packer` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_raw(data: *const c_char, len: usize, packer: *mut PackerBuffer) {
    let mut pos = 0usize;
    while pos < len {
        let p = unsafe { &mut *packer };
        let remaining = mpack_remaining(p);
        let to_copy = (len - pos).min(remaining);
        if to_copy > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.add(pos).cast::<u8>(),
                    p.ptr.cast::<u8>(),
                    to_copy,
                );
                p.ptr = p.ptr.add(to_copy);
            }
            pos += to_copy;
        }
        if pos < len
            && let Some(flush) = p.packer_flush
        {
            unsafe {
                flush(packer);
            }
        }
    }
    unsafe {
        mpack_check_buffer(packer);
    }
}

/// Pack a msgpack ext payload into `packer` (`mpack_ext`).
///
/// # Safety
/// `buf` must be valid for `len` bytes when `len > 0`, and `packer` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_ext(
    buf: *mut c_char,
    len: usize,
    ext_type: i8,
    packer: *mut PackerBuffer,
) {
    let p = unsafe { &mut *packer };
    unsafe {
        if len == 1 {
            mpack_w(&mut p.ptr, 0xd4);
        } else if len == 2 {
            mpack_w(&mut p.ptr, 0xd5);
        } else if len <= 0xff {
            mpack_w(&mut p.ptr, 0xc7);
        } else if len < 0xffff {
            mpack_w(&mut p.ptr, 0xc8);
            mpack_w2(&mut p.ptr, len as u32);
        } else if len < 0xffff_ffff {
            mpack_w(&mut p.ptr, 0xc9);
            mpack_w4(&mut p.ptr, len as u32);
        } else {
            std::process::abort();
        }
        mpack_w(&mut p.ptr, ext_type as u8);
        mpack_raw(buf, len, packer);
    }
}

/// Pack a Neovim handle (Buffer/Window/Tabpage) as a msgpack ext object (`mpack_handle`).
///
/// # Safety
/// `packer` must be a valid writable pointer to a `PackerBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_handle(
    obj_type: ObjectType,
    handle: handle_T,
    packer: *mut PackerBuffer,
) {
    let exttype = ((obj_type as c_int) - EXT_OBJECT_TYPE_SHIFT) as u8;
    let p = unsafe { &mut *packer };
    unsafe {
        if (-0x1f..=0x7f).contains(&handle) {
            mpack_w(&mut p.ptr, 0xd4);
            mpack_w(&mut p.ptr, exttype);
            mpack_w(&mut p.ptr, handle as u8);
        } else {
            assert!(handle >= 0);
            let mut buf = [0 as c_char; MPACK_ITEM_SIZE];
            let mut pos = buf.as_mut_ptr();
            mpack_uint(&mut pos, handle as u32);
            let packsize = (pos as usize) - (buf.as_ptr() as usize);
            mpack_w(&mut p.ptr, 0xc7);
            mpack_w(&mut p.ptr, packsize as u8);
            mpack_w(&mut p.ptr, exttype);
            std::ptr::copy_nonoverlapping(buf.as_ptr().cast::<u8>(), p.ptr.cast::<u8>(), packsize);
            p.ptr = p.ptr.add(packsize);
        }
    }
}

/// Pack `obj` into `packer` (`mpack_object`).
///
/// # Safety
/// `obj` and `packer` must be valid writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_object(obj: *mut Object, packer: *mut PackerBuffer) {
    unsafe {
        mpack_object_inner(obj, std::ptr::null_mut(), 0, packer);
    }
}

/// Pack `arr` into `packer` (`mpack_object_array`).
///
/// # Safety
/// `arr` and `packer` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_object_array(arr: Array, packer: *mut PackerBuffer) {
    let p = unsafe { &mut *packer };
    unsafe {
        mpack_array(&mut p.ptr, arr.size as u32);
        if arr.size > 0 {
            let mut container = Object {
                obj_type: ObjectType::kObjectTypeArray,
                data: ObjectData { array: arr },
            };
            let cont_ptr = if arr.size > 1 {
                &mut container
            } else {
                std::ptr::null_mut()
            };
            mpack_object_inner(arr.items, cont_ptr, 1, packer);
        }
    }
}

/// Pack `current` and any remaining elements in `container` into `packer` (`mpack_object_inner`).
///
/// # Safety
/// `current` and `packer` must be valid pointers, and `container` must be null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mpack_object_inner(
    mut current: *mut Object,
    mut container: *mut Object,
    mut container_idx: usize,
    packer: *mut PackerBuffer,
) {
    let mut stack = std::mem::MaybeUninit::<KVecWithInit<ContainerStackItem, 2>>::zeroed();
    let stack = unsafe {
        let s = &mut *stack.as_mut_ptr();
        s.init();
        s
    };

    loop {
        unsafe {
            mpack_check_buffer(packer);
            let cur = &mut *current;
            match cur.obj_type {
                ObjectType::kObjectTypeLuaRef => {
                    let repr = nlua_funcref_str(cur.data.luaref, std::ptr::null_mut(), true);
                    api_free_luaref(cur.data.luaref);
                    cur.data.luaref = LUA_NOREF;
                    let s = if repr.is_null() {
                        NvimString {
                            data: std::ptr::null_mut(),
                            size: 0,
                        }
                    } else {
                        NvimString {
                            data: repr,
                            size: CStr::from_ptr(repr).to_bytes().len(),
                        }
                    };
                    mpack_str(s, packer);
                    xfree(repr.cast::<c_void>());
                }
                ObjectType::kObjectTypeUnset | ObjectType::kObjectTypeNil => {
                    mpack_nil(&mut (*packer).ptr);
                }
                ObjectType::kObjectTypeBoolean => {
                    mpack_bool(&mut (*packer).ptr, cur.data.boolean);
                }
                ObjectType::kObjectTypeInteger => {
                    mpack_integer(&mut (*packer).ptr, cur.data.integer);
                }
                ObjectType::kObjectTypeFloat => {
                    mpack_float8(&mut (*packer).ptr, cur.data.floating);
                }
                ObjectType::kObjectTypeString => {
                    mpack_str(cur.data.string, packer);
                }
                ObjectType::kObjectTypeBuffer
                | ObjectType::kObjectTypeWindow
                | ObjectType::kObjectTypeTabpage => {
                    mpack_handle(cur.obj_type, cur.data.integer as handle_T, packer);
                }
                ObjectType::kObjectTypeDict | ObjectType::kObjectTypeArray => {
                    let current_size = if cur.obj_type == ObjectType::kObjectTypeArray {
                        let sz = cur.data.array.size;
                        mpack_array(&mut (*packer).ptr, sz as u32);
                        sz
                    } else {
                        let sz = cur.data.dict.size;
                        mpack_map(&mut (*packer).ptr, sz as u32);
                        sz
                    };
                    if current_size > 0 {
                        if cur.obj_type == ObjectType::kObjectTypeArray && current_size == 1 {
                            current = cur.data.array.items;
                            continue;
                        }
                        if !container.is_null() {
                            stack.push(ContainerStackItem {
                                container,
                                idx: container_idx,
                            });
                        }
                        container = current;
                        container_idx = 0;
                    }
                }
            }

            if container.is_null() {
                if stack.size > 0 {
                    stack.size -= 1;
                    let it = *stack.items.add(stack.size);
                    container = it.container;
                    container_idx = it.idx;
                } else {
                    break;
                }
            }

            let cont = &mut *container;
            if cont.obj_type == ObjectType::kObjectTypeArray {
                let arr = cont.data.array;
                current = arr.items.add(container_idx);
                container_idx += 1;
                if container_idx >= arr.size {
                    container = std::ptr::null_mut();
                }
            } else {
                let dict = cont.data.dict;
                let it = &mut *dict.items.add(container_idx);
                container_idx += 1;
                mpack_check_buffer(packer);
                mpack_str(it.key, packer);
                current = &mut it.value;
                if container_idx >= dict.size {
                    container = std::ptr::null_mut();
                }
            }
        }
    }

    unsafe {
        stack.destroy();
    }
}

unsafe extern "C" fn flush_string_buffer(buffer: *mut PackerBuffer) {
    let b = unsafe { &mut *buffer };
    let current_capacity = (b.endptr as usize) - (b.startptr as usize);
    let new_capacity = 2 * current_capacity;
    let len = (b.ptr as usize) - (b.startptr as usize);

    b.startptr = unsafe { xrealloc(b.startptr.cast::<c_void>(), new_capacity).cast::<c_char>() };
    b.ptr = unsafe { b.startptr.add(len) };
    b.endptr = unsafe { b.startptr.add(new_capacity) };
}

/// Create a growable `PackerBuffer` backed by `xmalloc` (`packer_string_buffer`).
#[unsafe(no_mangle)]
pub extern "C" fn packer_string_buffer() -> PackerBuffer {
    const INITIAL_SIZE: usize = 64;
    let alloc = unsafe { xmalloc(INITIAL_SIZE).cast::<c_char>() };
    PackerBuffer {
        startptr: alloc,
        ptr: alloc,
        endptr: unsafe { alloc.add(INITIAL_SIZE) },
        anydata: std::ptr::null_mut(),
        anyint: 0,
        packer_flush: Some(flush_string_buffer),
    }
}

/// Extract the packed `String` from a `packer_string_buffer` (`packer_take_string`).
///
/// # Safety
/// `buffer` must be a valid pointer to a `PackerBuffer` created by `packer_string_buffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn packer_take_string(buffer: *mut PackerBuffer) -> NvimString {
    let b = unsafe { &*buffer };
    NvimString {
        data: b.startptr,
        size: (b.ptr as usize) - (b.startptr as usize),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_integers_floats_and_strings() {
        let mut pb = packer_string_buffer();
        unsafe {
            mpack_integer(&mut pb.ptr, -32);
            mpack_integer(&mut pb.ptr, -33);
            mpack_integer(&mut pb.ptr, 42);
            let s = c"hello";
            mpack_str(
                NvimString {
                    data: s.as_ptr().cast_mut(),
                    size: 5,
                },
                &mut pb,
            );
            let out = packer_take_string(&mut pb);
            let slice = std::slice::from_raw_parts(out.data.cast::<u8>(), out.size);
            assert_eq!(
                slice,
                &[0xe0, 0xd0, 0xdf, 42, 0xa5, b'h', b'e', b'l', b'l', b'o']
            );
            xfree(out.data.cast::<c_void>());
        }
    }
}
