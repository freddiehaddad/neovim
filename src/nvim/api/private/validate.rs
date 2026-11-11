#![forbid(unsafe_code)]

//! API argument validation helpers (`src/nvim/api/private/validate.c`).

use std::rc::Rc;

pub struct CloneCell<T>(std::cell::RefCell<T>);

impl<T> CloneCell<T> {
    #[inline]
    pub const fn new(val: T) -> Self {
        Self(std::cell::RefCell::new(val))
    }

    #[inline]
    pub fn set(&self, val: T) {
        *self.0.borrow_mut() = val;
    }

    #[inline]
    pub fn replace(&self, val: T) -> T {
        self.0.replace(val)
    }
}

impl<T: Clone> CloneCell<T> {
    #[inline]
    pub fn get(&self) -> T {
        self.0.borrow().clone()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NvimStringData {
    #[default]
    Null,
    Static(&'static [u8]),
    Owned(Rc<Vec<u8>>),
}

impl NvimStringData {
    #[inline]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct NvimString {
    pub data: NvimStringData,
    pub size: usize,
}

impl NvimString {
    #[inline]
    pub const fn new() -> Self {
        Self {
            data: NvimStringData::Null,
            size: 0,
        }
    }

    #[inline]
    pub const fn empty_cstr() -> Self {
        Self {
            data: NvimStringData::Static(b"\0"),
            size: 0,
        }
    }

    #[inline]
    pub const fn from_static_cstr(s: &'static std::ffi::CStr) -> Self {
        Self {
            data: NvimStringData::Static(s.to_bytes_with_nul()),
            size: s.to_bytes().len(),
        }
    }

    #[inline]
    pub const fn from_static_bytes_len(bytes: &'static [u8], size: usize) -> Self {
        Self {
            data: NvimStringData::Static(if bytes.is_empty() { b"\0" } else { bytes }),
            size,
        }
    }

    #[inline]
    pub fn from_cstr(s: &std::ffi::CStr) -> Self {
        Self::from_bytes(s.to_bytes())
    }

    #[inline]
    pub fn from_opt_cstr(s: Option<&std::ffi::CStr>) -> Self {
        match s {
            Some(s) => Self::from_cstr(s),
            None => Self::new(),
        }
    }

    #[inline]
    pub fn zeroed(size: usize) -> Self {
        Self {
            data: NvimStringData::Owned(Rc::new(vec![0u8; size + 1])),
            size,
        }
    }

    #[inline]
    pub fn from_bytes_len(bytes: &[u8], size: usize) -> Self {
        let copy_len = size.min(bytes.len());
        let mut v = vec![0u8; size + 1];
        v[..copy_len].copy_from_slice(&bytes[..copy_len]);
        Self {
            data: NvimStringData::Owned(Rc::new(v)),
            size,
        }
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_bytes_len(bytes, bytes.len())
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.data {
            NvimStringData::Null => b"",
            NvimStringData::Static(s) => &s[..self.size.min(s.len())],
            NvimStringData::Owned(v) => &v[..self.size.min(v.len())],
        }
    }

    #[inline]
    pub fn slice(&self, len: usize) -> &[u8] {
        let b = self.as_bytes();
        &b[..len.min(b.len())]
    }

    #[inline]
    pub fn as_bytes_with_nul_mut(&mut self) -> &mut [u8] {
        if matches!(self.data, NvimStringData::Null) {
            return &mut [];
        }
        if let NvimStringData::Static(s) = self.data {
            let mut v = vec![0u8; (self.size + 1).max(s.len())];
            v[..s.len()].copy_from_slice(s);
            self.data = NvimStringData::Owned(Rc::new(v));
        }
        match &mut self.data {
            NvimStringData::Owned(rc) => {
                let v = Rc::make_mut(rc);
                if v.len() < self.size + 1 {
                    v.resize(self.size + 1, 0);
                }
                &mut v[..self.size + 1]
            }
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn cstr_bytes(&self) -> &[u8] {
        if self.data.is_null() {
            b""
        } else if self.size == 0 {
            self.as_cstr().to_bytes()
        } else {
            crate::memory::bytes_until_nul(self.as_bytes())
        }
    }

    #[inline]
    pub fn opt_bytes(&self) -> Option<&[u8]> {
        if self.data.is_null() {
            None
        } else {
            Some(self.cstr_bytes())
        }
    }

    #[inline]
    pub fn first_byte(&self) -> u8 {
        self.cstr_bytes().first().copied().unwrap_or(0)
    }

    #[inline]
    pub fn as_cstr(&self) -> &std::ffi::CStr {
        self.as_opt_cstr().unwrap_or(c"")
    }

    #[inline]
    pub fn as_opt_cstr(&self) -> Option<&std::ffi::CStr> {
        let raw: &[u8] = match &self.data {
            NvimStringData::Null => return None,
            NvimStringData::Static(s) => s,
            NvimStringData::Owned(v) => v.as_slice(),
        };
        Some(crate::memory::bytes_as_cstr(raw))
    }

    #[inline]
    pub fn to_cstring(&self) -> std::ffi::CString {
        self.as_cstr().to_owned()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorType {
    ErrorTypeNone = -1,
    ErrorTypeException = 0,
    ErrorTypeValidation = 1,
}

impl ErrorType {
    #[inline]
    pub const fn from_raw(v: i32) -> Option<Self> {
        match v {
            -1 => Some(Self::ErrorTypeNone),
            0 => Some(Self::ErrorTypeException),
            1 => Some(Self::ErrorTypeValidation),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Error {
    pub err_type: ErrorType,
    pub msg: Option<std::ffi::CString>,
}

impl Error {
    #[inline]
    pub const fn new() -> Self {
        Self {
            err_type: ErrorType::ErrorTypeNone,
            msg: None,
        }
    }

    #[inline]
    pub fn msg_cstr(&self) -> Option<&std::ffi::CStr> {
        self.msg.as_deref()
    }
}

impl Default for Error {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectType {
    ObjectTypeNil = 0,
    ObjectTypeBoolean = 1,
    ObjectTypeInteger = 2,
    ObjectTypeFloat = 3,
    ObjectTypeString = 4,
    ObjectTypeArray = 5,
    ObjectTypeDict = 6,
    ObjectTypeLuaRef = 7,
    ObjectTypeUnset = 8,
    ObjectTypeBuffer = 9,
    ObjectTypeWindow = 10,
    ObjectTypeTabpage = 11,
}

impl ObjectType {
    #[inline]
    pub const fn from_raw(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::ObjectTypeNil),
            1 => Some(Self::ObjectTypeBoolean),
            2 => Some(Self::ObjectTypeInteger),
            3 => Some(Self::ObjectTypeFloat),
            4 => Some(Self::ObjectTypeString),
            5 => Some(Self::ObjectTypeArray),
            6 => Some(Self::ObjectTypeDict),
            7 => Some(Self::ObjectTypeLuaRef),
            8 => Some(Self::ObjectTypeUnset),
            9 => Some(Self::ObjectTypeBuffer),
            10 => Some(Self::ObjectTypeWindow),
            11 => Some(Self::ObjectTypeTabpage),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct Array {
    pub size: usize,
    pub capacity: usize,
    pub items: Option<Rc<Vec<Object>>>,
}

impl Array {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: None,
        }
    }

    #[inline(always)]
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            size: 0,
            capacity: max_size,
            items: if max_size == 0 {
                None
            } else {
                Some(Rc::new(vec![Object::new(); max_size]))
            },
        }
    }

    #[inline(always)]
    pub fn from_slice(slice: &mut [Object]) -> Self {
        let len = slice.len();
        Self {
            size: len,
            capacity: len,
            items: if len == 0 {
                None
            } else {
                Some(Rc::new(slice.iter_mut().map(std::mem::take).collect()))
            },
        }
    }

    #[inline(always)]
    pub fn from_buf(buf: &mut [Object]) -> Self {
        Self::with_capacity(buf.len())
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[Object] {
        match &self.items {
            Some(rc) => &rc[..self.size.min(rc.len())],
            None => &[],
        }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [Object] {
        if self.size == 0 {
            return &mut [];
        }
        let rc = self
            .items
            .get_or_insert_with(|| Rc::new(vec![Object::new(); self.size]));
        let v = Rc::make_mut(rc);
        if v.len() < self.size {
            v.resize(self.size, Object::new());
        }
        &mut v[..self.size]
    }

    #[inline(always)]
    pub fn next_slot_mut(&mut self) -> &mut Object {
        let idx = self.size;
        &mut self[idx]
    }

    #[inline]
    pub fn push(&mut self, value: Object) {
        let idx = self.size;
        self.size += 1;
        if self.capacity < self.size {
            self.capacity = self.size.next_power_of_two().max(8);
        }
        let rc = self
            .items
            .get_or_insert_with(|| Rc::new(Vec::with_capacity(self.capacity)));
        let v = Rc::make_mut(rc);
        if idx < v.len() {
            v[idx] = value;
        } else {
            if v.len() < idx {
                v.resize(idx, Object::new());
            }
            v.push(value);
        }
    }
}

impl std::ops::Index<usize> for Array {
    type Output = Object;

    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        &self.items.as_ref().unwrap()[index]
    }
}

impl std::ops::IndexMut<usize> for Array {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let need_len = (index + 1).max(self.capacity).max(self.size);
        let rc = self
            .items
            .get_or_insert_with(|| Rc::new(vec![Object::new(); need_len]));
        let v = Rc::make_mut(rc);
        if v.len() <= index {
            v.resize(index + 1, Object::new());
        }
        &mut v[index]
    }
}

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct Dict {
    pub size: usize,
    pub capacity: usize,
    pub items: Option<Rc<Vec<KeyValuePair>>>,
}

impl Dict {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: None,
        }
    }

    #[inline(always)]
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            size: 0,
            capacity: max_size,
            items: if max_size == 0 {
                None
            } else {
                Some(Rc::new(vec![KeyValuePair::new(); max_size]))
            },
        }
    }

    #[inline(always)]
    pub fn from_slice(slice: &mut [KeyValuePair]) -> Self {
        let len = slice.len();
        Self {
            size: len,
            capacity: len,
            items: if len == 0 {
                None
            } else {
                Some(Rc::new(slice.iter_mut().map(std::mem::take).collect()))
            },
        }
    }

    #[inline(always)]
    pub fn from_buf(buf: &mut [KeyValuePair]) -> Self {
        Self::with_capacity(buf.len())
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[KeyValuePair] {
        match &self.items {
            Some(rc) => &rc[..self.size.min(rc.len())],
            None => &[],
        }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [KeyValuePair] {
        if self.size == 0 {
            return &mut [];
        }
        let rc = self
            .items
            .get_or_insert_with(|| Rc::new(vec![KeyValuePair::new(); self.size]));
        let v = Rc::make_mut(rc);
        if v.len() < self.size {
            v.resize(self.size, KeyValuePair::new());
        }
        &mut v[..self.size]
    }

    #[inline(always)]
    pub fn next_slot_mut(&mut self) -> &mut KeyValuePair {
        let idx = self.size;
        &mut self[idx]
    }

    #[inline]
    pub fn push(&mut self, value: KeyValuePair) {
        let idx = self.size;
        self.size += 1;
        if self.capacity < self.size {
            self.capacity = self.size.next_power_of_two().max(8);
        }
        let rc = self
            .items
            .get_or_insert_with(|| Rc::new(Vec::with_capacity(self.capacity)));
        let v = Rc::make_mut(rc);
        if idx < v.len() {
            v[idx] = value;
        } else {
            if v.len() < idx {
                v.resize(idx, KeyValuePair::new());
            }
            v.push(value);
        }
    }
}

impl std::ops::Index<usize> for Dict {
    type Output = KeyValuePair;

    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        &self.items.as_ref().unwrap()[index]
    }
}

impl std::ops::IndexMut<usize> for Dict {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let need_len = (index + 1).max(self.capacity).max(self.size);
        let rc = self
            .items
            .get_or_insert_with(|| Rc::new(vec![KeyValuePair::new(); need_len]));
        let v = Rc::make_mut(rc);
        if v.len() <= index {
            v.resize(index + 1, KeyValuePair::new());
        }
        &mut v[index]
    }
}

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub enum ObjectData {
    #[default]
    Nil,
    Boolean(bool),
    Integer(i64),
    Floating(f64),
    String(NvimString),
    Array(Array),
    Dict(Dict),
    LuaRef(i32),
}

impl ObjectData {
    #[inline]
    pub const fn boolean(&self) -> bool {
        match *self {
            Self::Boolean(b) => b,
            Self::Integer(i) => i != 0,
            _ => false,
        }
    }

    #[inline]
    pub const fn integer(&self) -> i64 {
        match *self {
            Self::Integer(i) => i,
            Self::Boolean(b) => b as i64,
            Self::LuaRef(r) => r as i64,
            _ => 0,
        }
    }

    #[inline]
    pub fn integer_mut(&mut self) -> &mut i64 {
        match self {
            Self::Integer(i) => i,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub const fn floating(&self) -> f64 {
        match *self {
            Self::Floating(f) => f,
            Self::Integer(i) => i as f64,
            _ => 0.0,
        }
    }

    #[inline]
    pub fn string(&self) -> NvimString {
        match self {
            Self::String(s) => s.clone(),
            _ => NvimString::new(),
        }
    }

    #[inline]
    pub fn as_cstr(&self) -> &std::ffi::CStr {
        match self {
            Self::String(s) => s.as_cstr(),
            _ => c"",
        }
    }

    #[inline]
    pub fn as_opt_cstr(&self) -> Option<&std::ffi::CStr> {
        match self {
            Self::String(s) => s.as_opt_cstr(),
            _ => None,
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::String(s) => s.as_bytes(),
            _ => &[],
        }
    }

    #[inline]
    pub fn cstr_bytes(&self) -> &[u8] {
        match self {
            Self::String(s) => s.cstr_bytes(),
            _ => &[0],
        }
    }

    #[inline]
    pub fn string_mut(&mut self) -> &mut NvimString {
        match self {
            Self::String(s) => s,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub fn array(&self) -> Array {
        match self {
            Self::Array(a) => a.clone(),
            _ => Array::new(),
        }
    }

    #[inline]
    pub fn array_slice(&self) -> &[Object] {
        match self {
            Self::Array(a) => a.as_slice(),
            _ => &[],
        }
    }

    #[inline]
    pub fn array_mut(&mut self) -> &mut Array {
        match self {
            Self::Array(a) => a,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub fn dict(&self) -> Dict {
        match self {
            Self::Dict(d) => d.clone(),
            _ => Dict::new(),
        }
    }

    #[inline]
    pub fn dict_slice(&self) -> &[KeyValuePair] {
        match self {
            Self::Dict(d) => d.as_slice(),
            _ => &[],
        }
    }

    #[inline]
    pub fn dict_mut(&mut self) -> &mut Dict {
        match self {
            Self::Dict(d) => d,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub const fn luaref(&self) -> i32 {
        match *self {
            Self::LuaRef(r) => r,
            Self::Integer(i) => i as i32,
            _ => 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug)]
pub struct Object {
    pub obj_type: ObjectType,
    pub data: ObjectData,
}

impl Object {
    #[inline]
    pub const fn new() -> Self {
        Self {
            obj_type: ObjectType::ObjectTypeNil,
            data: ObjectData::Nil,
        }
    }
}

impl Default for Object {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct KeyValuePair {
    pub key: NvimString,
    pub value: Object,
}

impl KeyValuePair {
    #[inline]
    pub const fn new() -> Self {
        Self {
            key: NvimString::new(),
            value: Object::new(),
        }
    }
}

pub trait KeyDict: 'static {
    fn is_set_mask(&self) -> u64;
    fn mark_set_bit(&mut self, bit: u32);
}

pub enum KeyField<T: 'static> {
    None,
    Object(fn(&T) -> &Object, fn(&mut T) -> &mut Object),
    Integer(fn(&T) -> &i64, fn(&mut T) -> &mut i64),
    Float(fn(&T) -> &f64, fn(&mut T) -> &mut f64),
    Boolean(fn(&T) -> &bool, fn(&mut T) -> &mut bool),
    String(fn(&T) -> &NvimString, fn(&mut T) -> &mut NvimString),
    Array(fn(&T) -> &Array, fn(&mut T) -> &mut Array),
    Dict(fn(&T) -> &Dict, fn(&mut T) -> &mut Dict),
    I32(fn(&T) -> &i32, fn(&mut T) -> &mut i32),
    StringArray(
        fn(&T) -> &Vec<NvimString>,
        fn(&mut T) -> &mut Vec<NvimString>,
    ),
}

impl<T: 'static> Clone for KeyField<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for KeyField<T> {}

pub trait KeyFieldType<T: 'static>: Clone {
    fn get_ref<'a>(field: &KeyField<T>, target: &'a T) -> &'a Self;
    fn get_mut<'a>(field: &KeyField<T>, target: &'a mut T) -> &'a mut Self;
}

macro_rules! impl_key_field_type {
    ($ty:ty, $variant:ident) => {
        impl<T: 'static> KeyFieldType<T> for $ty {
            #[inline]
            fn get_ref<'a>(field: &KeyField<T>, target: &'a T) -> &'a Self {
                match *field {
                    KeyField::$variant(get, _) => get(target),
                    _ => unreachable!(),
                }
            }

            #[inline]
            fn get_mut<'a>(field: &KeyField<T>, target: &'a mut T) -> &'a mut Self {
                match *field {
                    KeyField::$variant(_, get_mut) => get_mut(target),
                    _ => unreachable!(),
                }
            }
        }
    };
}

impl_key_field_type!(Object, Object);
impl_key_field_type!(i64, Integer);
impl_key_field_type!(f64, Float);
impl_key_field_type!(bool, Boolean);
impl_key_field_type!(NvimString, String);
impl_key_field_type!(Array, Array);
impl_key_field_type!(Dict, Dict);
impl_key_field_type!(i32, I32);
impl_key_field_type!(Vec<NvimString>, StringArray);

#[repr(C)]
pub struct KeySetLink<T: 'static> {
    pub str: Option<&'static std::ffi::CStr>,
    pub field: KeyField<T>,
    pub key_type: i32,
    pub opt_index: i32,
    pub is_hlgroup: bool,
}

impl<T: 'static> Clone for KeySetLink<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for KeySetLink<T> {}

impl<T: KeyDict> KeySetLink<T> {
    #[inline]
    pub fn mark_set(&self, retval: &mut T) {
        if self.opt_index >= 0 {
            retval.mark_set_bit(self.opt_index as u32);
        }
    }

    #[inline]
    pub fn is_field_set(&self, value: &T) -> bool {
        if self.opt_index >= 0 {
            (value.is_set_mask() & (1u64 << (self.opt_index as u32))) != 0
        } else {
            true
        }
    }

    #[inline]
    pub fn field_mut<'a, F: KeyFieldType<T>>(&self, retval: &'a mut T) -> &'a mut F {
        F::get_mut(&self.field, retval)
    }

    #[inline]
    pub fn field_copy<F: KeyFieldType<T>>(&self, value: &T) -> F {
        F::get_ref(&self.field, value).clone()
    }
}

pub const fn object_typename(t: ObjectType) -> &'static str {
    match t {
        ObjectType::ObjectTypeUnset => "unset",
        ObjectType::ObjectTypeNil => "nil",
        ObjectType::ObjectTypeBoolean => "Boolean",
        ObjectType::ObjectTypeInteger => "Integer",
        ObjectType::ObjectTypeFloat => "Float",
        ObjectType::ObjectTypeString => "String",
        ObjectType::ObjectTypeArray => "Array",
        ObjectType::ObjectTypeDict => "Dict",
        ObjectType::ObjectTypeLuaRef => "Function",
        ObjectType::ObjectTypeBuffer => "Buffer",
        ObjectType::ObjectTypeWindow => "Window",
        ObjectType::ObjectTypeTabpage => "Tabpage",
    }
}

fn set_error_str(err: &mut Error, err_type: ErrorType, msg: &str) {
    let bytes = msg.as_bytes();
    let len = bytes.len().min((1024 * 1024) - 1);
    err.msg = Some(crate::memory::bytes_to_cstring(&bytes[..len]));
    err.err_type = err_type;
}

#[inline]
/// Create `"Invalid …"` validation message on `err` (`api_err_invalid`).
pub fn api_err_invalid(
    err: &mut Error,
    name: &std::ffi::CStr,
    val_s: Option<&std::ffi::CStr>,
    val_n: i64,
    quote_val: bool,
) {
    let name_str = name.to_string_lossy();
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = match val_s {
        Some(vs) if vs.to_bytes().is_empty() => {
            if has_space {
                format!("Invalid {name_str}")
            } else {
                format!("Invalid '{name_str}'")
            }
        }
        None => {
            if has_space {
                format!("Invalid {name_str}: {val_n}")
            } else {
                format!("Invalid '{name_str}': {val_n}")
            }
        }
        Some(vs) => {
            let val_str = vs.to_string_lossy();
            match (has_space, quote_val) {
                (true, true) => format!("Invalid {name_str}: '{val_str}'"),
                (true, false) => format!("Invalid {name_str}: {val_str}"),
                (false, true) => format!("Invalid '{name_str}': '{val_str}'"),
                (false, false) => format!("Invalid '{name_str}': {val_str}"),
            }
        }
    };

    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Create `"Invalid …: expected …"` validation message on `err` (`api_err_exp`).
pub fn api_err_exp(
    err: &mut Error,
    name: &std::ffi::CStr,
    expected: &std::ffi::CStr,
    actual: Option<&std::ffi::CStr>,
) {
    let name_str = name.to_string_lossy();
    let exp_str = expected.to_string_lossy();
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = match actual {
        None => {
            if has_space {
                format!("Invalid {name_str}: expected {exp_str}")
            } else {
                format!("Invalid '{name_str}': expected {exp_str}")
            }
        }
        Some(actual) => {
            let act_str = actual.to_string_lossy();
            if has_space {
                format!("Invalid {name_str}: expected {exp_str}, got {act_str}")
            } else {
                format!("Invalid '{name_str}': expected {exp_str}, got {act_str}")
            }
        }
    };

    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Create `"Required: …"` validation message on `err` (`api_err_required`).
pub fn api_err_required(err: &mut Error, name: &std::ffi::CStr) {
    let name_str = name.to_string_lossy();
    let has_space = name_str.as_bytes().contains(&b' ');
    let msg = if has_space {
        format!("Required: {name_str}")
    } else {
        format!("Required: '{name_str}'")
    };
    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Create `"Conflict: … not allowed with …"` validation message on `err` (`api_err_conflict`).
pub fn api_err_conflict(err: &mut Error, name: &std::ffi::CStr, name2: &std::ffi::CStr) {
    let name_str = name.to_string_lossy();
    let name2_str = name2.to_string_lossy();
    let has_space2 = name2_str.as_bytes().contains(&b' ');
    let msg = if has_space2 {
        format!("Conflict: '{name_str}' not allowed with {name2_str}")
    } else {
        format!("Conflict: '{name_str}' not allowed with '{name2_str}'")
    };
    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Validate that `arr` contains only `String` items (and optionally no embedded newlines)
/// (`check_string_array`).
pub fn check_string_array(
    arr: Array,
    name: &std::ffi::CStr,
    disallow_nl: bool,
    err: &mut Error,
) -> bool {
    let name_str = name.to_string_lossy();
    for i in 0..arr.size {
        let item = &arr[i];
        if item.obj_type != ObjectType::ObjectTypeString {
            let msg = format!(
                "Invalid '{name_str}' item: expected String, got {}",
                object_typename(item.obj_type)
            );
            set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
            return false;
        }
        if disallow_nl {
            let l = item.data.string();
            if l.as_bytes().contains(&b'\n') {
                let msg = format!("'{name_str}' item contains newlines");
                set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
                return false;
            }
        }
    }
    true
}

#[inline]
pub fn api_err_range(err: &mut Error, name: &std::ffi::CStr) {
    api_err_invalid(err, name, Some(c"out of range"), 0, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::helpers::api_clear_error;

    #[test]
    fn formats_validation_error_messages() {
        let mut err = Error::new();

        api_err_invalid(&mut err, c"col", None, -1, false);
        assert_eq!(err.err_type, ErrorType::ErrorTypeValidation);
        assert_eq!(
            err.msg.as_ref().unwrap().to_str().unwrap(),
            "Invalid 'col': -1"
        );
        api_clear_error(&mut err);

        api_err_exp(&mut err, c"buffer id", c"Integer", Some(c"String"));
        assert_eq!(
            err.msg.as_ref().unwrap().to_str().unwrap(),
            "Invalid buffer id: expected Integer, got String"
        );
        api_clear_error(&mut err);

        api_err_required(&mut err, c"width");
        assert_eq!(
            err.msg.as_ref().unwrap().to_str().unwrap(),
            "Required: 'width'"
        );
        api_clear_error(&mut err);

        api_err_conflict(&mut err, c"row", c"win");
        assert_eq!(
            err.msg.as_ref().unwrap().to_str().unwrap(),
            "Conflict: 'row' not allowed with 'win'"
        );
        api_clear_error(&mut err);
    }
}
