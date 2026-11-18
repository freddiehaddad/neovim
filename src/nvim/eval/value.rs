//! Vimscript value kinds and their payloads.
#![forbid(unsafe_code)]

use std::ffi::{CStr, CString};

use super::typval::{
    BlobId, DictId, ListId, PartialId, TvBlob, TvDict, TvList, TvPartial, VarType,
};

#[derive(Clone)]
pub struct InlineText {
    bytes: [u8; 22],
    len: u8,
}

impl InlineText {
    #[inline]
    fn from_clean_bytes(bytes: &[u8]) -> Self {
        debug_assert!(!bytes.contains(&0));
        let mut buffer = [0; 22];
        buffer[..bytes.len()].copy_from_slice(bytes);
        Self {
            bytes: buffer,
            len: bytes.len() as u8,
        }
    }

    #[inline]
    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    #[inline]
    fn as_cstr(&self) -> &CStr {
        CStr::from_bytes_with_nul(&self.bytes[..=self.len as usize]).unwrap()
    }
}

#[derive(Clone, Default)]
pub enum TvValue {
    #[default]
    Unknown,
    Number(i64),
    String(Option<CString>),
    Func(Option<CString>),
    List(ListId),
    Dict(DictId),
    Float(f64),
    Bool(bool),
    Null,
    Partial(PartialId),
    Blob(BlobId),
    InlineString(InlineText),
    InlineFunc(InlineText),
}

impl TvValue {
    pub const NULL_STRING: Self = Self::String(None);
    pub const NULL_FUNC: Self = Self::Func(None);

    #[inline]
    pub const fn var_type(&self) -> VarType {
        match self {
            Self::Unknown => VarType::VarUnknown,
            Self::Number(_) => VarType::VarNumber,
            Self::Bool(_) => VarType::VarBool,
            Self::Null => VarType::VarSpecial,
            Self::Float(_) => VarType::VarFloat,
            Self::String(_) | Self::InlineString(_) => VarType::VarString,
            Self::Func(_) | Self::InlineFunc(_) => VarType::VarFunc,
            Self::List(_) => VarType::VarList,
            Self::Dict(_) => VarType::VarDict,
            Self::Blob(_) => VarType::VarBlob,
            Self::Partial(_) => VarType::VarPartial,
        }
    }

    #[inline]
    pub const fn empty(kind: VarType) -> Self {
        match kind {
            VarType::VarUnknown => Self::Unknown,
            VarType::VarNumber => Self::Number(0),
            VarType::VarBool => Self::Bool(false),
            VarType::VarSpecial => Self::Null,
            VarType::VarFloat => Self::Float(0.0),
            VarType::VarString => Self::NULL_STRING,
            VarType::VarFunc => Self::NULL_FUNC,
            VarType::VarList => Self::List(ListId::NULL),
            VarType::VarDict => Self::Dict(DictId::NULL),
            VarType::VarBlob => Self::Blob(BlobId::NULL),
            VarType::VarPartial => Self::Partial(PartialId::NULL),
        }
    }

    #[inline]
    pub fn from_bytes_no_nul(bytes: &[u8]) -> Self {
        if bytes.len() <= 21 {
            return Self::from_bytes(bytes);
        }
        // Reserve the terminator up front instead of growing the allocation.
        let mut text = Vec::with_capacity(bytes.len() + 1);
        text.extend_from_slice(bytes);
        text.push(0);
        let text = CString::from_vec_with_nul(text)
            .unwrap_or_else(|error| crate::memory::bytes_to_cstring(&error.into_bytes()));
        Self::String(Some(text))
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let bytes = crate::memory::bytes_until_nul(bytes);
        if bytes.len() <= 21 {
            Self::InlineString(InlineText::from_clean_bytes(bytes))
        } else {
            Self::from_bytes_no_nul(bytes)
        }
    }

    #[inline]
    pub fn from_opt_bytes(bytes: Option<&[u8]>) -> Self {
        bytes.map_or(Self::NULL_STRING, Self::from_bytes)
    }

    #[inline]
    pub fn from_cstr(value: &CStr) -> Self {
        if value.to_bytes().len() <= 21 {
            Self::InlineString(InlineText::from_clean_bytes(value.to_bytes()))
        } else {
            Self::String(Some(value.to_owned()))
        }
    }

    #[inline]
    pub fn from_opt_cstr(value: Option<&CStr>) -> Self {
        value.map_or(Self::NULL_STRING, Self::from_cstr)
    }

    #[inline]
    pub fn from_cstring(value: CString) -> Self {
        if value.as_bytes().len() <= 21 {
            Self::from_cstr(&value)
        } else {
            Self::String(Some(value))
        }
    }

    #[inline]
    pub fn from_opt_cstring(value: Option<CString>) -> Self {
        value.map_or(Self::NULL_STRING, Self::from_cstring)
    }

    #[track_caller]
    #[inline]
    pub fn into_funcref(self) -> Self {
        match self {
            Self::String(name) | Self::Func(name) => Self::Func(name),
            Self::InlineString(name) | Self::InlineFunc(name) => Self::InlineFunc(name),
            _ => panic!("expected function name"),
        }
    }

    #[inline]
    pub fn take_text(&mut self) -> Option<CString> {
        match self {
            Self::String(value) | Self::Func(value) => value.take(),
            Self::InlineString(value) => {
                let result = value.as_cstr().to_owned();
                *self = Self::NULL_STRING;
                Some(result)
            }
            Self::InlineFunc(value) => {
                let result = value.as_cstr().to_owned();
                *self = Self::NULL_FUNC;
                Some(result)
            }
            _ => None,
        }
    }

    #[inline]
    pub const fn as_number(&self) -> Option<i64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    #[track_caller]
    #[inline]
    pub const fn number(&self) -> i64 {
        match self {
            Self::Number(value) => *value,
            _ => panic!("expected Number"),
        }
    }

    #[track_caller]
    #[inline]
    pub fn number_mut(&mut self) -> &mut i64 {
        match self {
            Self::Number(value) => value,
            _ => panic!("expected Number"),
        }
    }

    #[track_caller]
    #[inline]
    pub const fn boolean(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            _ => panic!("expected Boolean"),
        }
    }

    #[track_caller]
    #[inline]
    pub const fn float(&self) -> f64 {
        match self {
            Self::Float(value) => *value,
            _ => panic!("expected Float"),
        }
    }

    #[track_caller]
    #[inline]
    pub fn float_mut(&mut self) -> &mut f64 {
        match self {
            Self::Float(value) => value,
            _ => panic!("expected Float"),
        }
    }

    #[track_caller]
    #[inline]
    fn text(&self) -> Option<&CStr> {
        match self {
            Self::String(value) | Self::Func(value) => value.as_deref(),
            Self::InlineString(value) | Self::InlineFunc(value) => Some(value.as_cstr()),
            _ => panic!("expected String or Funcref"),
        }
    }

    #[inline]
    pub fn string_is_null(&self) -> bool {
        match self {
            Self::String(value) | Self::Func(value) => value.is_none(),
            Self::InlineString(_) | Self::InlineFunc(_) => false,
            _ => panic!("expected String or Funcref"),
        }
    }

    #[inline]
    pub fn string_cstr(&self) -> Option<&CStr> {
        match self {
            Self::String(value) | Self::Func(value) => value.as_deref(),
            Self::InlineString(value) | Self::InlineFunc(value) => Some(value.as_cstr()),
            _ => None,
        }
    }

    #[inline]
    pub fn string_cstr_or_empty(&self) -> &CStr {
        self.text().unwrap_or(c"")
    }

    #[inline]
    pub fn string_bytes(&self) -> &[u8] {
        match self {
            Self::String(value) | Self::Func(value) => {
                value.as_ref().map_or(&[], CString::as_bytes)
            }
            Self::InlineString(value) | Self::InlineFunc(value) => value.as_bytes(),
            _ => panic!("expected String or Funcref"),
        }
    }

    #[inline]
    pub fn string_opt_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(value) | Self::Func(value) => value.as_ref().map(CString::as_bytes),
            Self::InlineString(value) | Self::InlineFunc(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    #[inline]
    pub fn string_is_empty(&self) -> bool {
        self.string_bytes().is_empty()
    }

    #[inline]
    pub fn string_first_byte(&self) -> u8 {
        self.string_bytes().first().copied().unwrap_or(0)
    }

    #[inline]
    pub fn string_owned(&self) -> CString {
        self.string_cstr_or_empty().to_owned()
    }

    #[track_caller]
    #[inline]
    pub fn list(&self) -> ListId {
        match self {
            Self::List(value) => value.clone(),
            _ => panic!("expected List, got {:?}", self.var_type()),
        }
    }

    #[inline]
    pub fn list_ref(&self) -> &TvList {
        self.list_opt().expect("null list")
    }

    #[inline]
    pub fn list_opt(&self) -> Option<&TvList> {
        match self {
            Self::List(value) => value.opt(),
            _ => None,
        }
    }

    #[track_caller]
    #[inline]
    pub fn dict(&self) -> DictId {
        match self {
            Self::Dict(value) => value.clone(),
            _ => panic!("expected Dictionary"),
        }
    }

    #[inline]
    pub fn dict_ref(&self) -> &TvDict {
        self.dict_opt().expect("null dict")
    }

    #[inline]
    pub fn dict_opt(&self) -> Option<&TvDict> {
        match self {
            Self::Dict(value) => value.opt(),
            _ => None,
        }
    }

    #[track_caller]
    #[inline]
    pub fn blob(&self) -> BlobId {
        match self {
            Self::Blob(value) => value.clone(),
            _ => panic!("expected Blob"),
        }
    }

    #[inline]
    pub fn blob_ref(&self) -> &TvBlob {
        self.blob_opt().expect("null blob")
    }

    #[inline]
    pub fn blob_opt(&self) -> Option<&TvBlob> {
        match self {
            Self::Blob(value) => value.opt(),
            _ => None,
        }
    }

    #[track_caller]
    #[inline]
    pub fn partial(&self) -> PartialId {
        match self {
            Self::Partial(value) => value.clone(),
            _ => panic!("expected Partial"),
        }
    }

    #[inline]
    pub fn partial_opt(&self) -> Option<&TvPartial> {
        match self {
            Self::Partial(value) => value.opt(),
            _ => None,
        }
    }

    #[inline]
    pub fn any_addr(&self) -> usize {
        match self {
            Self::String(value) | Self::Func(value) => {
                value.as_ref().map_or(0, |value| value.as_ptr() as usize)
            }
            Self::InlineString(value) | Self::InlineFunc(value) => value.bytes.as_ptr() as usize,
            Self::List(value) => value.as_ptr() as usize,
            Self::Dict(value) => value.as_ptr() as usize,
            Self::Blob(value) => value
                .opt()
                .map_or(0, |value| std::ptr::from_ref(value) as usize),
            Self::Partial(value) => value
                .opt()
                .map_or(0, |value| std::ptr::from_ref(value) as usize),
            _ => 0,
        }
    }
}
