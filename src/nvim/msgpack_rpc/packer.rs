//! Msgpack-RPC serializer (`src/nvim/msgpack_rpc/packer.c`).

use crate::api::private::validate::{Array, NvimString, Object, ObjectData, ObjectType};
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::{api_free_luaref, nlua_funcref_str};
use std::ffi::{c_char, c_int};

pub const MPACK_ITEM_SIZE: usize = 9;
pub(crate) const EXT_OBJECT_TYPE_SHIFT: c_int = ObjectType::ObjectTypeBuffer as c_int;

pub type ObjHandle = c_int;
pub type Integer = i64;

#[derive(Clone, Copy, Debug)]
pub enum PackerData {
    None,
    StringBuffer,
    Ui(*mut crate::msgpack_rpc::channel::RemoteUI),
    File(*mut crate::os::fileio::FileDescriptor),
    Chans(*mut *mut crate::msgpack_rpc::channel::Channel),
}

impl PackerData {
    #[inline]
    pub fn as_ui(self) -> &'static mut crate::msgpack_rpc::channel::RemoteUI {
        match self {
            Self::Ui(p) => unsafe { &mut *p },
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn as_file(self) -> &'static mut crate::os::fileio::FileDescriptor {
        match self {
            Self::File(p) => unsafe { &mut *p },
            _ => unreachable!(),
        }
    }

    #[inline]
    pub const fn as_chans(self) -> *mut *mut crate::msgpack_rpc::channel::Channel {
        match self {
            Self::Chans(p) => p,
            _ => std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug)]
pub struct PackerBuffer {
    pub startptr: *mut c_char,
    pub ptr: *mut c_char,
    pub endptr: *mut c_char,
    pub anydata: PackerData,
    pub anyint: i64,
    pub owned_buf: Vec<c_char>,
}

impl PackerBuffer {
    pub const fn new() -> Self {
        Self {
            startptr: std::ptr::null_mut(),
            ptr: std::ptr::null_mut(),
            endptr: std::ptr::null_mut(),
            anydata: PackerData::None,
            anyint: 0,
            owned_buf: Vec::new(),
        }
    }
}

pub fn packer_flush_buffer(packer: &mut PackerBuffer) {
    match packer.anydata {
        PackerData::None => {}
        PackerData::StringBuffer => flush_string_buffer(packer),
        PackerData::Ui(_) => crate::api::ui::ui_flush_callback(packer),
        PackerData::File(_) => crate::shada::flush_file_buffer(packer),
        PackerData::Chans(_) => crate::msgpack_rpc::channel::channel_flush_callback(packer),
    }
}

impl Default for PackerBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ContainerStackItem {
    container: *mut Object,
    idx: usize,
}

#[inline]
pub(crate) fn mpack_w(b: *mut *mut c_char, byte: u8) {
    unsafe {
        **b = byte as c_char;
        *b = (*b).add(1);
    }
}

#[inline]
pub fn mpack_w2(b: *mut *mut c_char, v: u32) {
    let bytes = (v as u16).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*b).cast::<u8>(), 2);
        *b = (*b).add(2);
    }
}

#[inline]
fn mpack_w4(b: *mut *mut c_char, v: u32) {
    let bytes = v.to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*b).cast::<u8>(), 4);
        *b = (*b).add(4);
    }
}

#[inline]
fn mpack_w8(b: *mut *mut c_char, v: u64) {
    let bytes = v.to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*b).cast::<u8>(), 8);
        *b = (*b).add(8);
    }
}

#[inline]
pub(crate) fn mpack_uint(buf: *mut *mut c_char, val: u32) {
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

#[inline]
pub(crate) fn mpack_nil(buf: *mut *mut c_char) {
    mpack_w(buf, 0xc0);
}

#[inline]
pub(crate) fn mpack_bool(buf: *mut *mut c_char, val: bool) {
    mpack_w(buf, 0xc2 | u8::from(val));
}

#[inline]
pub(crate) fn mpack_array(buf: *mut *mut c_char, len: u32) {
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

#[inline]
pub(crate) fn mpack_map(buf: *mut *mut c_char, len: u32) {
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

#[inline]
fn mpack_remaining(packer: &PackerBuffer) -> usize {
    (packer.endptr as usize).saturating_sub(packer.ptr as usize)
}

/// Ensure at least `2 * MPACK_ITEM_SIZE` bytes remain in `packer` (`mpack_check_buffer`).
///
/// # Safety
/// `packer` must be a valid writable pointer to a `PackerBuffer`.
pub fn mpack_check_buffer(packer: &mut PackerBuffer) {
    if mpack_remaining(packer) < 2 * MPACK_ITEM_SIZE {
        packer_flush_buffer(packer);
    }
}

/// Pack a 64-bit unsigned integer into `*ptr` (`mpack_uint64`).
///
/// # Safety
/// `ptr` and `*ptr` must be valid writable pointers with sufficient space.
pub fn mpack_uint64(ptr: *mut *mut c_char, i: u64) {
    if i > 0x0fff_ffff {
        mpack_w(ptr, 0xcf);
        mpack_w8(ptr, i);
    } else {
        mpack_uint(ptr, i as u32);
    }
}

/// Pack a signed 64-bit integer into `*ptr` (`mpack_integer`).
///
/// # Safety
/// `ptr` and `*ptr` must be valid writable pointers with sufficient space.
pub fn mpack_integer(ptr: *mut *mut c_char, i: Integer) {
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

/// Pack an IEEE 754 `f64` into `*ptr` (`mpack_float8`).
///
/// # Safety
/// `ptr` and `*ptr` must be valid writable pointers with sufficient space.
pub fn mpack_float8(ptr: *mut *mut c_char, i: f64) {
    mpack_w(ptr, 0xcb);
    mpack_w8(ptr, i.to_bits());
}

/// Pack a msgpack string into `packer` (`mpack_str`).
///
/// # Safety
/// `str_val` and `packer` must be valid.
pub fn mpack_str(str_val: NvimString, packer: &mut PackerBuffer) {
    let len = str_val.size;
    if len < 32 {
        mpack_w(&mut packer.ptr, 0xa0 | (len as u8));
    } else if len < 0xff {
        mpack_w(&mut packer.ptr, 0xd9);
        mpack_w(&mut packer.ptr, len as u8);
    } else if len < 0xffff {
        mpack_w(&mut packer.ptr, 0xda);
        mpack_w2(&mut packer.ptr, len as u32);
    } else if len < 0xffff_ffff {
        mpack_w(&mut packer.ptr, 0xdb);
        mpack_w4(&mut packer.ptr, len as u32);
    } else {
        std::process::abort();
    }
    mpack_raw(str_val.data, len, packer);
}

/// Pack a msgpack binary blob into `packer` (`mpack_bin`).
///
/// # Safety
/// `str_val` and `packer` must be valid.
pub fn mpack_bin(str_val: NvimString, packer: &mut PackerBuffer) {
    let len = str_val.size;
    if len < 0xff {
        mpack_w(&mut packer.ptr, 0xc4);
        mpack_w(&mut packer.ptr, len as u8);
    } else if len < 0xffff {
        mpack_w(&mut packer.ptr, 0xc5);
        mpack_w2(&mut packer.ptr, len as u32);
    } else if len < 0xffff_ffff {
        mpack_w(&mut packer.ptr, 0xc6);
        mpack_w4(&mut packer.ptr, len as u32);
    } else {
        std::process::abort();
    }
    mpack_raw(str_val.data, len, packer);
}

/// Write raw bytes `data[..len]` into `packer`, flushing as needed (`mpack_raw`).
///
/// # Safety
/// `data` must be valid for `len` bytes when `len > 0`, and `packer` must be valid.
pub fn mpack_raw(data: impl crate::strings::IntoCCharPtr, len: usize, packer: &mut PackerBuffer) {
    let data = data.into_c_char_ptr();
    let mut pos = 0usize;
    while pos < len {
        let remaining = mpack_remaining(packer);
        let to_copy = (len - pos).min(remaining);
        if to_copy > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.add(pos).cast::<u8>(),
                    packer.ptr.cast::<u8>(),
                    to_copy,
                );
                packer.ptr = packer.ptr.add(to_copy);
            }
            pos += to_copy;
        }
        if pos < len {
            packer_flush_buffer(packer);
        }
    }
    mpack_check_buffer(packer);
}

/// Pack a msgpack ext payload into `packer` (`mpack_ext`).
///
/// # Safety
/// `buf` must be valid for `len` bytes when `len > 0`, and `packer` must be valid.
pub fn mpack_ext(
    buf: impl crate::strings::IntoCCharPtr,
    len: usize,
    ext_type: i8,
    packer: &mut PackerBuffer,
) {
    let buf = buf.into_c_char_ptr();
    if len == 1 {
        mpack_w(&mut packer.ptr, 0xd4);
    } else if len == 2 {
        mpack_w(&mut packer.ptr, 0xd5);
    } else if len <= 0xff {
        mpack_w(&mut packer.ptr, 0xc7);
    } else if len < 0xffff {
        mpack_w(&mut packer.ptr, 0xc8);
        mpack_w2(&mut packer.ptr, len as u32);
    } else if len < 0xffff_ffff {
        mpack_w(&mut packer.ptr, 0xc9);
        mpack_w4(&mut packer.ptr, len as u32);
    } else {
        std::process::abort();
    }
    mpack_w(&mut packer.ptr, ext_type as u8);
    mpack_raw(buf, len, packer);
}

/// Pack a Neovim handle (Buffer/Window/Tabpage) as a msgpack ext object (`mpack_handle`).
///
/// # Safety
/// `packer` must be a valid writable pointer to a `PackerBuffer`.
pub fn mpack_handle(obj_type: ObjectType, handle: ObjHandle, packer: &mut PackerBuffer) {
    let exttype = ((obj_type as c_int) - EXT_OBJECT_TYPE_SHIFT) as u8;
    unsafe {
        if (-0x1f..=0x7f).contains(&handle) {
            mpack_w(&mut packer.ptr, 0xd4);
            mpack_w(&mut packer.ptr, exttype);
            mpack_w(&mut packer.ptr, handle as u8);
        } else {
            assert!(handle >= 0);
            let mut buf = [0 as c_char; MPACK_ITEM_SIZE];
            let mut pos = buf.as_mut_ptr();
            mpack_uint(&mut pos, handle as u32);
            let packsize = (pos as usize) - (buf.as_ptr() as usize);
            mpack_w(&mut packer.ptr, 0xc7);
            mpack_w(&mut packer.ptr, packsize as u8);
            mpack_w(&mut packer.ptr, exttype);
            std::ptr::copy_nonoverlapping(
                buf.as_ptr().cast::<u8>(),
                packer.ptr.cast::<u8>(),
                packsize,
            );
            packer.ptr = packer.ptr.add(packsize);
        }
    }
}

/// Pack `obj` into `packer` (`mpack_object`).
pub fn mpack_object(obj: &mut Object, packer: &mut PackerBuffer) {
    mpack_object_inner(obj, std::ptr::null_mut(), 0, packer);
}

/// Pack `arr` into `packer` (`mpack_object_array`).
///
/// # Safety
/// `arr` and `packer` must be valid.
pub fn mpack_object_array(arr: Array, packer: &mut PackerBuffer) {
    mpack_array(&mut packer.ptr, arr.size as u32);
    if arr.size > 0 {
        let mut container = Object {
            obj_type: ObjectType::ObjectTypeArray,
            data: ObjectData::Array(arr),
        };
        let cont_ptr = if arr.size > 1 {
            &mut container
        } else {
            std::ptr::null_mut()
        };
        mpack_object_inner(arr.items, cont_ptr, 1, packer);
    }
}

/// Pack `current` and any remaining elements in `container` into `packer` (`mpack_object_inner`).
///
/// # Safety
/// `current` and `packer` must be valid pointers, and `container` must be null or valid.
pub fn mpack_object_inner(
    mut current: *mut Object,
    mut container: *mut Object,
    mut container_idx: usize,
    packer: &mut PackerBuffer,
) {
    let mut stack: Vec<ContainerStackItem> = Vec::with_capacity(2);

    loop {
        unsafe {
            mpack_check_buffer(packer);
            let cur = &mut *current;
            match cur.obj_type {
                ObjectType::ObjectTypeLuaRef => {
                    let cs = nlua_funcref_str(cur.data.luaref(), true);
                    api_free_luaref(cur.data.luaref());
                    cur.data = ObjectData::LuaRef(LUA_NOREF);
                    let s = NvimString {
                        data: cs.as_ptr().cast_mut(),
                        size: cs.to_bytes().len(),
                    };
                    mpack_str(s, packer);
                }
                ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {
                    mpack_nil(&mut packer.ptr);
                }
                ObjectType::ObjectTypeBoolean => {
                    mpack_bool(&mut packer.ptr, cur.data.boolean());
                }
                ObjectType::ObjectTypeInteger => {
                    mpack_integer(&mut packer.ptr, cur.data.integer());
                }
                ObjectType::ObjectTypeFloat => {
                    mpack_float8(&mut packer.ptr, cur.data.floating());
                }
                ObjectType::ObjectTypeString => {
                    mpack_str(cur.data.string(), packer);
                }
                ObjectType::ObjectTypeBuffer
                | ObjectType::ObjectTypeWindow
                | ObjectType::ObjectTypeTabpage => {
                    mpack_handle(cur.obj_type, cur.data.integer() as ObjHandle, packer);
                }
                ObjectType::ObjectTypeDict | ObjectType::ObjectTypeArray => {
                    let current_size = if cur.obj_type == ObjectType::ObjectTypeArray {
                        let sz = cur.data.array().size;
                        mpack_array(&mut packer.ptr, sz as u32);
                        sz
                    } else {
                        let sz = cur.data.dict().size;
                        mpack_map(&mut packer.ptr, sz as u32);
                        sz
                    };
                    if current_size > 0 {
                        if cur.obj_type == ObjectType::ObjectTypeArray && current_size == 1 {
                            current = cur.data.array().items;
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
                if let Some(it) = stack.pop() {
                    container = it.container;
                    container_idx = it.idx;
                } else {
                    break;
                }
            }

            let cont = &mut *container;
            if cont.obj_type == ObjectType::ObjectTypeArray {
                let arr = cont.data.array();
                current = arr.items.add(container_idx);
                container_idx += 1;
                if container_idx >= arr.size {
                    container = std::ptr::null_mut();
                }
            } else {
                let mut dict = cont.data.dict();
                let it = &mut dict[container_idx];
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
}

fn flush_string_buffer(buffer: &mut PackerBuffer) {
    let current_capacity = (buffer.endptr as usize) - (buffer.startptr as usize);
    let new_capacity = 2 * current_capacity;
    let len = (buffer.ptr as usize) - (buffer.startptr as usize);

    buffer.owned_buf.resize(new_capacity, 0);
    buffer.startptr = buffer.owned_buf.as_mut_ptr();
    buffer.ptr = unsafe { buffer.startptr.add(len) };
    buffer.endptr = unsafe { buffer.startptr.add(new_capacity) };
}

/// Create a growable `PackerBuffer` backed by `Vec<c_char>` (`packer_string_buffer`).
pub fn packer_string_buffer() -> PackerBuffer {
    const INITIAL_SIZE: usize = 64;
    let mut owned_buf = vec![0 as c_char; INITIAL_SIZE];
    let alloc = owned_buf.as_mut_ptr();
    PackerBuffer {
        startptr: alloc,
        ptr: alloc,
        endptr: unsafe { alloc.add(INITIAL_SIZE) },
        anydata: PackerData::StringBuffer,
        anyint: 0,
        owned_buf,
    }
}

/// Extract the packed bytes from a `packer_string_buffer` (`packer_take_string`).
///
/// # Safety
/// `buffer` must be a valid pointer to a `PackerBuffer` created by `packer_string_buffer`.
pub fn packer_take_string(buffer: &mut PackerBuffer) -> Vec<u8> {
    let len = (buffer.ptr as usize) - (buffer.startptr as usize);
    let vec = std::mem::take(&mut buffer.owned_buf);
    buffer.startptr = std::ptr::null_mut();
    buffer.ptr = std::ptr::null_mut();
    buffer.endptr = std::ptr::null_mut();
    let mut bytes: Vec<u8> = vec.into_iter().map(|c| c as u8).collect();
    bytes.truncate(len);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_integers_floats_and_strings() {
        let mut pb = packer_string_buffer();
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
        let slice = packer_take_string(&mut pb);
        assert_eq!(
            slice,
            &[0xe0, 0xd0, 0xdf, 42, 0xa5, b'h', b'e', b'l', b'l', b'o']
        );
    }
}
