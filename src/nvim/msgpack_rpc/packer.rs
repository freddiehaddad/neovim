//! Msgpack-RPC serializer (`src/nvim/msgpack_rpc/packer.c`).
#![forbid(unsafe_code)]

use crate::api::private::validate::{Array, NvimString, Object, ObjectData, ObjectType};
use crate::lua::converter::LUA_NOREF;
use crate::lua::executor::{api_free_luaref, nlua_funcref_str};

pub const MPACK_ITEM_SIZE: usize = 9;
pub(crate) const EXT_OBJECT_TYPE_SHIFT: i32 = ObjectType::ObjectTypeBuffer as i32;

pub type ObjHandle = i32;
pub type Integer = i64;

#[derive(Debug)]
pub enum PackerData {
    None,
    StringBuffer,
    Ui(u64),
    File(Box<crate::os::fileio::FileDescriptor>),
    Chans,
}

impl PackerData {
    #[inline]
    pub fn as_file(&mut self) -> &mut crate::os::fileio::FileDescriptor {
        match self {
            Self::File(p) => p,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn take_file(&mut self) -> crate::os::fileio::FileDescriptor {
        match std::mem::replace(self, Self::None) {
            Self::File(p) => *p,
            _ => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct PackerBuffer {
    pub pos: usize,
    pub anydata: PackerData,
    pub anyint: i64,
    pub owned_buf: Vec<u8>,
    pub cur_event: Option<&'static std::ffi::CStr>,
    pub nevents_pos: Option<usize>,
    pub ncalls_pos: Option<usize>,
    pub nevents: u32,
    pub ncalls: u32,
    pub flushed_events: bool,
    pub incomplete_event: bool,
    pub ncells_pending: usize,
}

impl PackerBuffer {
    pub const fn new() -> Self {
        Self {
            pos: 0,
            anydata: PackerData::None,
            anyint: 0,
            owned_buf: Vec::new(),
            cur_event: None,
            nevents_pos: None,
            ncalls_pos: None,
            nevents: 0,
            ncalls: 0,
            flushed_events: false,
            incomplete_event: false,
            ncells_pending: 0,
        }
    }
}

pub fn packer_flush_buffer(packer: &mut PackerBuffer) {
    match packer.anydata {
        PackerData::None => {}
        PackerData::StringBuffer => flush_string_buffer(packer),
        PackerData::Ui(_) => crate::api::ui::ui_flush_callback(packer),
        PackerData::File(_) => crate::shada::flush_file_buffer(packer),
        PackerData::Chans => crate::msgpack_rpc::channel::channel_flush_callback(packer),
    }
}

impl Default for PackerBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub(crate) fn mpack_w(packer: &mut PackerBuffer, byte: u8) {
    packer.owned_buf[packer.pos] = byte;
    packer.pos += 1;
}

#[inline]
pub fn mpack_w2(packer: &mut PackerBuffer, v: u32) {
    let bytes = (v as u16).to_be_bytes();
    packer.owned_buf[packer.pos..packer.pos + 2].copy_from_slice(&bytes);
    packer.pos += 2;
}

#[inline]
pub fn mpack_w2_at(packer: &mut PackerBuffer, offset: usize, v: u32) {
    let bytes = (v as u16).to_be_bytes();
    packer.owned_buf[offset..offset + 2].copy_from_slice(&bytes);
}

#[inline]
fn mpack_w4(packer: &mut PackerBuffer, v: u32) {
    let bytes = v.to_be_bytes();
    packer.owned_buf[packer.pos..packer.pos + 4].copy_from_slice(&bytes);
    packer.pos += 4;
}

#[inline]
fn mpack_w8(packer: &mut PackerBuffer, v: u64) {
    let bytes = v.to_be_bytes();
    packer.owned_buf[packer.pos..packer.pos + 8].copy_from_slice(&bytes);
    packer.pos += 8;
}

#[inline]
pub(crate) fn mpack_uint(buf: &mut PackerBuffer, val: u32) {
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
pub(crate) fn mpack_nil(buf: &mut PackerBuffer) {
    mpack_w(buf, 0xc0);
}

#[inline]
pub(crate) fn mpack_bool(buf: &mut PackerBuffer, val: bool) {
    mpack_w(buf, 0xc2 | u8::from(val));
}

#[inline]
pub(crate) fn mpack_array(buf: &mut PackerBuffer, len: u32) {
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
pub(crate) fn mpack_map(buf: &mut PackerBuffer, len: u32) {
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
    packer.owned_buf.len().saturating_sub(packer.pos)
}

/// Ensure at least `2 * MPACK_ITEM_SIZE` bytes remain in `packer` (`mpack_check_buffer`).
pub fn mpack_check_buffer(packer: &mut PackerBuffer) {
    if mpack_remaining(packer) < 2 * MPACK_ITEM_SIZE {
        packer_flush_buffer(packer);
    }
}

/// Pack a 64-bit unsigned integer into `packer` (`mpack_uint64`).
pub fn mpack_uint64(packer: &mut PackerBuffer, i: u64) {
    if i > 0x0fff_ffff {
        mpack_w(packer, 0xcf);
        mpack_w8(packer, i);
    } else {
        mpack_uint(packer, i as u32);
    }
}

/// Pack a signed 64-bit integer into `packer` (`mpack_integer`).
pub fn mpack_integer(packer: &mut PackerBuffer, i: Integer) {
    if i >= 0 {
        mpack_uint64(packer, i as u64);
    } else if i < -0x8000_0000 {
        mpack_w(packer, 0xd3);
        mpack_w8(packer, i as u64);
    } else if i < -0x8000 {
        mpack_w(packer, 0xd2);
        mpack_w4(packer, i as u32);
    } else if i < -0x80 {
        mpack_w(packer, 0xd1);
        mpack_w2(packer, i as u32);
    } else if i < -0x20 {
        mpack_w(packer, 0xd0);
        mpack_w(packer, i as u8);
    } else {
        mpack_w(packer, i as u8);
    }
}

/// Pack an IEEE 754 `f64` into `packer` (`mpack_float8`).
pub fn mpack_float8(packer: &mut PackerBuffer, i: f64) {
    mpack_w(packer, 0xcb);
    mpack_w8(packer, i.to_bits());
}

/// Pack a msgpack string into `packer` (`mpack_str`).
pub fn mpack_str(str_val: NvimString, packer: &mut PackerBuffer) {
    let len = str_val.size;
    if len < 32 {
        mpack_w(packer, 0xa0 | (len as u8));
    } else if len < 0xff {
        mpack_w(packer, 0xd9);
        mpack_w(packer, len as u8);
    } else if len < 0xffff {
        mpack_w(packer, 0xda);
        mpack_w2(packer, len as u32);
    } else if len < 0xffff_ffff {
        mpack_w(packer, 0xdb);
        mpack_w4(packer, len as u32);
    } else {
        std::process::abort();
    }
    let slice = str_val.slice(len);
    mpack_raw(slice, packer);
}

/// Pack a msgpack binary blob into `packer` (`mpack_bin`).
pub fn mpack_bin(str_val: NvimString, packer: &mut PackerBuffer) {
    let len = str_val.size;
    if len < 0xff {
        mpack_w(packer, 0xc4);
        mpack_w(packer, len as u8);
    } else if len < 0xffff {
        mpack_w(packer, 0xc5);
        mpack_w2(packer, len as u32);
    } else if len < 0xffff_ffff {
        mpack_w(packer, 0xc6);
        mpack_w4(packer, len as u32);
    } else {
        std::process::abort();
    }
    let slice = str_val.slice(len);
    mpack_raw(slice, packer);
}

/// Write raw `bytes` into `packer`, flushing as needed (`mpack_raw`).
pub fn mpack_raw(bytes: &[u8], packer: &mut PackerBuffer) {
    let len = bytes.len();
    let mut pos = 0usize;
    while pos < len {
        let remaining = mpack_remaining(packer);
        let to_copy = (len - pos).min(remaining);
        if to_copy > 0 {
            packer.owned_buf[packer.pos..packer.pos + to_copy]
                .copy_from_slice(&bytes[pos..pos + to_copy]);

            packer.pos += to_copy;
            pos += to_copy;
        }
        if pos < len {
            packer_flush_buffer(packer);
        }
    }
    mpack_check_buffer(packer);
}

/// Pack a msgpack ext payload into `packer` (`mpack_ext`).
pub fn mpack_ext(bytes: &[u8], ext_type: i8, packer: &mut PackerBuffer) {
    let len = bytes.len();
    if len == 1 {
        mpack_w(packer, 0xd4);
    } else if len == 2 {
        mpack_w(packer, 0xd5);
    } else if len <= 0xff {
        mpack_w(packer, 0xc7);
    } else if len < 0xffff {
        mpack_w(packer, 0xc8);
        mpack_w2(packer, len as u32);
    } else if len < 0xffff_ffff {
        mpack_w(packer, 0xc9);
        mpack_w4(packer, len as u32);
    } else {
        std::process::abort();
    }
    mpack_w(packer, ext_type as u8);
    mpack_raw(bytes, packer);
}

/// Pack a Neovim handle (Buffer/Window/Tabpage) as a msgpack ext object (`mpack_handle`).
pub fn mpack_handle(obj_type: ObjectType, handle: ObjHandle, packer: &mut PackerBuffer) {
    let exttype = ((obj_type as i32) - EXT_OBJECT_TYPE_SHIFT) as u8;
    if (-0x1f..=0x7f).contains(&handle) {
        mpack_w(packer, 0xd4);
        mpack_w(packer, exttype);
        mpack_w(packer, handle as u8);
    } else {
        assert!(handle >= 0);
        let mut tmp = PackerBuffer {
            owned_buf: vec![0u8; MPACK_ITEM_SIZE],
            ..PackerBuffer::new()
        };
        mpack_uint(&mut tmp, handle as u32);
        let packsize = tmp.pos;
        mpack_w(packer, 0xc7);
        mpack_w(packer, packsize as u8);
        mpack_w(packer, exttype);

        packer.owned_buf[packer.pos..packer.pos + packsize]
            .copy_from_slice(&tmp.owned_buf[..packsize]);

        packer.pos += packsize;
    }
}

/// Pack `obj` into `packer` (`mpack_object`).
pub fn mpack_object(cur: &mut Object, packer: &mut PackerBuffer) {
    mpack_check_buffer(packer);
    match cur.obj_type {
        ObjectType::ObjectTypeLuaRef => {
            let cs = nlua_funcref_str(cur.data.luaref(), true);
            api_free_luaref(cur.data.luaref());
            cur.data = ObjectData::LuaRef(LUA_NOREF);
            mpack_str(NvimString::from_cstr(&cs), packer);
        }
        ObjectType::ObjectTypeUnset | ObjectType::ObjectTypeNil => {
            mpack_nil(packer);
        }
        ObjectType::ObjectTypeBoolean => {
            mpack_bool(packer, cur.data.boolean());
        }
        ObjectType::ObjectTypeInteger => {
            mpack_integer(packer, cur.data.integer());
        }
        ObjectType::ObjectTypeFloat => {
            mpack_float8(packer, cur.data.floating());
        }
        ObjectType::ObjectTypeString => {
            mpack_str(cur.data.string(), packer);
        }
        ObjectType::ObjectTypeBuffer
        | ObjectType::ObjectTypeWindow
        | ObjectType::ObjectTypeTabpage => {
            mpack_handle(cur.obj_type, cur.data.integer() as ObjHandle, packer);
        }
        ObjectType::ObjectTypeArray => {
            let mut arr = cur.data.array();
            mpack_array(packer, arr.size as u32);
            for item in arr.as_mut_slice() {
                mpack_object(item, packer);
            }
        }
        ObjectType::ObjectTypeDict => {
            let mut dict = cur.data.dict();
            mpack_map(packer, dict.size as u32);
            for it in dict.as_mut_slice() {
                mpack_check_buffer(packer);
                mpack_str(it.key.clone(), packer);
                mpack_object(&mut it.value, packer);
            }
        }
    }
}

/// Pack `arr` into `packer` (`mpack_object_array`).
pub fn mpack_object_array(mut arr: Array, packer: &mut PackerBuffer) {
    mpack_array(packer, arr.size as u32);
    for item in arr.as_mut_slice() {
        mpack_object(item, packer);
    }
}

fn flush_string_buffer(buffer: &mut PackerBuffer) {
    let current_capacity = buffer.owned_buf.len();
    let new_capacity = 2 * current_capacity;
    buffer.owned_buf.resize(new_capacity, 0);
}

/// Create a growable `PackerBuffer` backed by `Vec<u8>` (`packer_string_buffer`).
pub fn packer_string_buffer() -> PackerBuffer {
    const INITIAL_SIZE: usize = 64;
    let owned_buf = vec![0u8; INITIAL_SIZE];
    PackerBuffer {
        anydata: PackerData::StringBuffer,
        owned_buf,
        ..PackerBuffer::new()
    }
}

/// Extract the packed bytes from a `packer_string_buffer` (`packer_take_string`).
///
/// # Safety
/// `buffer` must be a valid pointer to a `PackerBuffer` created by `packer_string_buffer`.
pub fn packer_take_string(buffer: &mut PackerBuffer) -> Vec<u8> {
    let len = buffer.pos;
    let mut bytes = std::mem::take(&mut buffer.owned_buf);
    buffer.pos = 0;
    bytes.truncate(len);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_integers_floats_and_strings() {
        let mut pb = packer_string_buffer();
        mpack_integer(&mut pb, -32);
        mpack_integer(&mut pb, -33);
        mpack_integer(&mut pb, 42);
        let s = c"hello";
        mpack_str(NvimString::from_cstr(s), &mut pb);
        let slice = packer_take_string(&mut pb);
        assert_eq!(
            slice,
            &[0xe0, 0xd0, 0xdf, 42, 0xa5, b'h', b'e', b'l', b'l', b'o']
        );
    }
}
