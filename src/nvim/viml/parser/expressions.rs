//! Vimscript expression lexer and parser (`src/nvim/viml/parser/expressions.c`).

use crate::charset::{
    Uvarnumber, ascii_isalpha, ascii_isdigit, ascii_iswhite, ascii_isxdigit, vim_str2nr,
};
use crate::keycodes::{FSK_IN_STRING, FSK_KEYCODE, FSK_SIMPLIFY, NUL, trans_special};
use crate::mbyte::{mb_copy_char, utf_char2bytes, utf_char2len, utfc_ptr2len_len};
use crate::viml::parser::parser::{
    ParserHighlightChunk, ParserLine, ParserPosition, ParserState, viml_parser_get_remaining_line,
};
use std::ffi::{c_char, c_int};

use crate::eval::STR2NR_ALL;
use crate::eval::typval::TvFloat;
use crate::testing::FF;

pub(crate) const BS: u8 = 0x08;
pub(crate) const TAB: u8 = 0x09;
pub(crate) const NL: u8 = 0x0a;
pub(crate) const CAR: u8 = 0x0d;
pub(crate) const ESC: u8 = 0x1b;
pub(crate) const AUTOLOAD_CHAR: u8 = b'#';

const CTRL_A_CONST: u8 = 1;
pub(crate) const CTRL_H_CONST: u8 = 8;
const CTRL_K_CONST_CONST: u8 = 11;
const CTRL_Z_CONST_CONST: u8 = 26;

pub type ExprCaseCompareStrategy = c_int;
pub const K_CCSTRATEGY_USE_OPTION: ExprCaseCompareStrategy = 0;
pub const K_CCSTRATEGY_MATCH_CASE: ExprCaseCompareStrategy = b'#' as c_int;
pub const K_CCSTRATEGY_IGNORE_CASE: ExprCaseCompareStrategy = b'?' as c_int;

pub type LexExprTokenType = c_int;
pub const K_EXPR_LEX_INVALID: LexExprTokenType = 0;
pub const K_EXPR_LEX_MISSING: LexExprTokenType = 1;
pub const K_EXPR_LEX_SPACING: LexExprTokenType = 2;
pub const K_EXPR_LEX_EOC: LexExprTokenType = 3;
pub const K_EXPR_LEX_QUESTION: LexExprTokenType = 4;
pub const K_EXPR_LEX_COLON: LexExprTokenType = 5;
pub const K_EXPR_LEX_OR: LexExprTokenType = 6;
pub const K_EXPR_LEX_AND: LexExprTokenType = 7;
pub const K_EXPR_LEX_COMPARISON: LexExprTokenType = 8;
pub const K_EXPR_LEX_PLUS: LexExprTokenType = 9;
pub const K_EXPR_LEX_MINUS: LexExprTokenType = 10;
pub const K_EXPR_LEX_DOT: LexExprTokenType = 11;
pub const K_EXPR_LEX_MULTIPLICATION: LexExprTokenType = 12;
pub const K_EXPR_LEX_NOT: LexExprTokenType = 13;
pub const K_EXPR_LEX_NUMBER: LexExprTokenType = 14;
pub const K_EXPR_LEX_SINGLE_QUOTED_STRING: LexExprTokenType = 15;
pub const K_EXPR_LEX_DOUBLE_QUOTED_STRING: LexExprTokenType = 16;
pub const K_EXPR_LEX_OPTION: LexExprTokenType = 17;
pub const K_EXPR_LEX_REGISTER: LexExprTokenType = 18;
pub const K_EXPR_LEX_ENV: LexExprTokenType = 19;
pub const K_EXPR_LEX_PLAIN_IDENTIFIER: LexExprTokenType = 20;
pub const K_EXPR_LEX_BRACKET: LexExprTokenType = 21;
pub const K_EXPR_LEX_FIGURE_BRACE: LexExprTokenType = 22;
pub const K_EXPR_LEX_PARENTHESIS: LexExprTokenType = 23;
pub const K_EXPR_LEX_COMMA: LexExprTokenType = 24;
pub const K_EXPR_LEX_ARROW: LexExprTokenType = 25;
pub const K_EXPR_LEX_ASSIGNMENT: LexExprTokenType = 26;

pub type ExprComparisonType = c_int;
pub const K_EXPR_CMP_EQUAL: ExprComparisonType = 0;
pub const K_EXPR_CMP_MATCHES: ExprComparisonType = 1;
pub const K_EXPR_CMP_GREATER: ExprComparisonType = 2;
pub const K_EXPR_CMP_GREATER_OR_EQUAL: ExprComparisonType = 3;
pub const K_EXPR_CMP_IDENTICAL: ExprComparisonType = 4;

pub type ExprOptScope = c_int;
pub const K_EXPR_OPT_SCOPE_UNSPECIFIED: ExprOptScope = 0;

pub type ExprAssignmentType = c_int;
pub const K_EXPR_ASGN_PLAIN: ExprAssignmentType = 0;
pub const K_EXPR_ASGN_ADD: ExprAssignmentType = 1;
pub const K_EXPR_ASGN_SUBTRACT: ExprAssignmentType = 2;
pub const K_EXPR_ASGN_CONCAT: ExprAssignmentType = 3;

pub type ExprVarScope = c_int;
pub const K_EXPR_VAR_SCOPE_MISSING: ExprVarScope = 0;

pub type LexExprMulType = c_int;
pub const K_EXPR_LEX_MUL_MUL: LexExprMulType = 0;
pub const K_EXPR_LEX_MUL_DIV: LexExprMulType = 1;
pub const K_EXPR_LEX_MUL_MOD: LexExprMulType = 2;

pub const K_ELFLAG_PEEK: c_int = 1 << 0;
pub const K_ELFLAG_FORBID_SCOPE: c_int = 1 << 1;
pub const K_ELFLAG_ALLOW_FLOAT: c_int = 1 << 2;
pub const K_ELFLAG_IS_NOT_CMP: c_int = 1 << 3;
pub const K_ELFLAG_FORBID_EOC: c_int = 1 << 4;

pub type ExprASTNodeType = c_int;
pub const K_EXPR_NODE_MISSING: ExprASTNodeType = 0;
pub const K_EXPR_NODE_OP_MISSING: ExprASTNodeType = 1;
pub const K_EXPR_NODE_TERNARY: ExprASTNodeType = 2;
pub const K_EXPR_NODE_TERNARY_VALUE: ExprASTNodeType = 3;
pub const K_EXPR_NODE_REGISTER: ExprASTNodeType = 4;
pub const K_EXPR_NODE_SUBSCRIPT: ExprASTNodeType = 5;
pub const K_EXPR_NODE_LIST_LITERAL: ExprASTNodeType = 6;
pub const K_EXPR_NODE_UNARY_PLUS: ExprASTNodeType = 7;
pub const K_EXPR_NODE_BINARY_PLUS: ExprASTNodeType = 8;
pub const K_EXPR_NODE_NESTED: ExprASTNodeType = 9;
pub const K_EXPR_NODE_CALL: ExprASTNodeType = 10;
pub const K_EXPR_NODE_PLAIN_IDENTIFIER: ExprASTNodeType = 11;
pub const K_EXPR_NODE_PLAIN_KEY: ExprASTNodeType = 12;
pub const K_EXPR_NODE_COMPLEX_IDENTIFIER: ExprASTNodeType = 13;
pub const K_EXPR_NODE_UNKNOWN_FIGURE: ExprASTNodeType = 14;
pub const K_EXPR_NODE_LAMBDA: ExprASTNodeType = 15;
pub const K_EXPR_NODE_DICT_LITERAL: ExprASTNodeType = 16;
pub const K_EXPR_NODE_CURLY_BRACES_IDENTIFIER: ExprASTNodeType = 17;
pub const K_EXPR_NODE_COMMA: ExprASTNodeType = 18;
pub const K_EXPR_NODE_COLON: ExprASTNodeType = 19;
pub const K_EXPR_NODE_ARROW: ExprASTNodeType = 20;
pub const K_EXPR_NODE_COMPARISON: ExprASTNodeType = 21;
pub const K_EXPR_NODE_CONCAT: ExprASTNodeType = 22;
pub const K_EXPR_NODE_CONCAT_OR_SUBSCRIPT: ExprASTNodeType = 23;
pub const K_EXPR_NODE_INTEGER: ExprASTNodeType = 24;
pub const K_EXPR_NODE_FLOAT: ExprASTNodeType = 25;
pub const K_EXPR_NODE_SINGLE_QUOTED_STRING: ExprASTNodeType = 26;
pub const K_EXPR_NODE_DOUBLE_QUOTED_STRING: ExprASTNodeType = 27;
pub const K_EXPR_NODE_OR: ExprASTNodeType = 28;
pub const K_EXPR_NODE_AND: ExprASTNodeType = 29;
pub const K_EXPR_NODE_UNARY_MINUS: ExprASTNodeType = 30;
pub const K_EXPR_NODE_BINARY_MINUS: ExprASTNodeType = 31;
pub const K_EXPR_NODE_NOT: ExprASTNodeType = 32;
pub const K_EXPR_NODE_MULTIPLICATION: ExprASTNodeType = 33;
pub const K_EXPR_NODE_DIVISION: ExprASTNodeType = 34;
pub const K_EXPR_NODE_MOD: ExprASTNodeType = 35;
pub const K_EXPR_NODE_OPTION: ExprASTNodeType = 36;
pub const K_EXPR_NODE_ENVIRONMENT: ExprASTNodeType = 37;
pub const K_EXPR_NODE_ASSIGNMENT: ExprASTNodeType = 38;

pub const K_EXPR_FLAGS_MULTI: c_int = 1 << 0;
pub const K_EXPR_FLAGS_DISALLOW_EOC: c_int = 1 << 1;
pub const K_EXPR_FLAGS_PARSE_LET: c_int = 1 << 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenCmp {
    pub r#type: ExprComparisonType,
    pub ccs: ExprCaseCompareStrategy,
    pub inv: bool,
}

impl LexExprTokenCmp {
    pub const fn new() -> Self {
        Self {
            r#type: 0,
            ccs: 0,
            inv: false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenMul {
    pub r#type: LexExprMulType,
}

impl LexExprTokenMul {
    pub const fn new() -> Self {
        Self { r#type: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenBrc {
    pub closing: bool,
}

impl LexExprTokenBrc {
    pub const fn new() -> Self {
        Self { closing: false }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenReg {
    pub name: c_int,
}

impl LexExprTokenReg {
    pub const fn new() -> Self {
        Self { name: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenStr {
    pub closed: bool,
}

impl LexExprTokenStr {
    pub const fn new() -> Self {
        Self { closed: false }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenOpt {
    pub name: *const c_char,
    pub len: usize,
    pub scope: ExprOptScope,
}

impl LexExprTokenOpt {
    pub const fn new() -> Self {
        Self {
            name: std::ptr::null(),
            len: 0,
            scope: 0,
        }
    }
}

impl Default for LexExprTokenOpt {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenVar {
    pub scope: ExprVarScope,
    pub autoload: bool,
}

impl LexExprTokenVar {
    pub const fn new() -> Self {
        Self {
            scope: 0,
            autoload: false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenErr {
    pub r#type: LexExprTokenType,
    pub msg: Option<&'static std::ffi::CStr>,
}

impl LexExprTokenErr {
    pub const fn new() -> Self {
        Self {
            r#type: 0,
            msg: None,
        }
    }
}

impl Default for LexExprTokenErr {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub enum LexExprTokenNumVal {
    Integer(Uvarnumber),
    Floating(TvFloat),
}

impl Default for LexExprTokenNumVal {
    fn default() -> Self {
        Self::new()
    }
}

impl LexExprTokenNumVal {
    pub const fn new() -> Self {
        Self::Integer(0)
    }

    #[inline]
    pub const fn integer(self) -> Uvarnumber {
        match self {
            Self::Integer(v) => v,
            Self::Floating(_) => 0,
        }
    }

    #[inline]
    pub const fn floating(self) -> TvFloat {
        match self {
            Self::Floating(v) => v,
            Self::Integer(_) => 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenNum {
    pub val: LexExprTokenNumVal,
    pub base: u8,
    pub is_float: bool,
}

impl LexExprTokenNum {
    pub const fn new() -> Self {
        Self {
            val: LexExprTokenNumVal::new(),
            base: 0,
            is_float: false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprTokenAss {
    pub r#type: ExprAssignmentType,
}

impl LexExprTokenAss {
    pub const fn new() -> Self {
        Self { r#type: 0 }
    }
}

#[derive(Clone, Copy, Default)]
pub enum LexExprTokenData {
    #[default]
    None,
    Cmp(LexExprTokenCmp),
    Mul(LexExprTokenMul),
    Brc(LexExprTokenBrc),
    Reg(LexExprTokenReg),
    Str(LexExprTokenStr),
    Opt(LexExprTokenOpt),
    Var(LexExprTokenVar),
    Err(LexExprTokenErr),
    Num(LexExprTokenNum),
    Ass(LexExprTokenAss),
}

impl LexExprTokenData {
    pub const fn new() -> Self {
        Self::None
    }

    #[inline]
    pub fn cmp(&self) -> &LexExprTokenCmp {
        static EMPTY: LexExprTokenCmp = LexExprTokenCmp::new();
        match self {
            Self::Cmp(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn cmp_mut(&mut self) -> &mut LexExprTokenCmp {
        if !matches!(self, Self::Cmp(_)) {
            *self = Self::Cmp(LexExprTokenCmp::new());
        }
        match self {
            Self::Cmp(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn mul(&self) -> &LexExprTokenMul {
        static EMPTY: LexExprTokenMul = LexExprTokenMul::new();
        match self {
            Self::Mul(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn mul_mut(&mut self) -> &mut LexExprTokenMul {
        if !matches!(self, Self::Mul(_)) {
            *self = Self::Mul(LexExprTokenMul::new());
        }
        match self {
            Self::Mul(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn brc(&self) -> &LexExprTokenBrc {
        static EMPTY: LexExprTokenBrc = LexExprTokenBrc::new();
        match self {
            Self::Brc(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn brc_mut(&mut self) -> &mut LexExprTokenBrc {
        if !matches!(self, Self::Brc(_)) {
            *self = Self::Brc(LexExprTokenBrc::new());
        }
        match self {
            Self::Brc(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn reg(&self) -> &LexExprTokenReg {
        static EMPTY: LexExprTokenReg = LexExprTokenReg::new();
        match self {
            Self::Reg(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn reg_mut(&mut self) -> &mut LexExprTokenReg {
        if !matches!(self, Self::Reg(_)) {
            *self = Self::Reg(LexExprTokenReg::new());
        }
        match self {
            Self::Reg(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn str(&self) -> &LexExprTokenStr {
        static EMPTY: LexExprTokenStr = LexExprTokenStr::new();
        match self {
            Self::Str(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn str_mut(&mut self) -> &mut LexExprTokenStr {
        if !matches!(self, Self::Str(_)) {
            *self = Self::Str(LexExprTokenStr::new());
        }
        match self {
            Self::Str(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn opt(&self) -> &LexExprTokenOpt {
        match self {
            Self::Opt(v) => v,
            _ => &const { LexExprTokenOpt::new() },
        }
    }

    #[inline]
    pub fn opt_mut(&mut self) -> &mut LexExprTokenOpt {
        if !matches!(self, Self::Opt(_)) {
            *self = Self::Opt(LexExprTokenOpt::new());
        }
        match self {
            Self::Opt(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn var(&self) -> &LexExprTokenVar {
        static EMPTY: LexExprTokenVar = LexExprTokenVar::new();
        match self {
            Self::Var(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn var_mut(&mut self) -> &mut LexExprTokenVar {
        if !matches!(self, Self::Var(_)) {
            *self = Self::Var(LexExprTokenVar::new());
        }
        match self {
            Self::Var(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn err(&self) -> &LexExprTokenErr {
        match self {
            Self::Err(v) => v,
            _ => &const { LexExprTokenErr::new() },
        }
    }

    #[inline]
    pub fn err_mut(&mut self) -> &mut LexExprTokenErr {
        if !matches!(self, Self::Err(_)) {
            *self = Self::Err(LexExprTokenErr::new());
        }
        match self {
            Self::Err(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn num(&self) -> &LexExprTokenNum {
        static EMPTY: LexExprTokenNum = LexExprTokenNum::new();
        match self {
            Self::Num(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn num_mut(&mut self) -> &mut LexExprTokenNum {
        if !matches!(self, Self::Num(_)) {
            *self = Self::Num(LexExprTokenNum::new());
        }
        match self {
            Self::Num(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn ass(&self) -> &LexExprTokenAss {
        static EMPTY: LexExprTokenAss = LexExprTokenAss::new();
        match self {
            Self::Ass(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn ass_mut(&mut self) -> &mut LexExprTokenAss {
        if !matches!(self, Self::Ass(_)) {
            *self = Self::Ass(LexExprTokenAss::new());
        }
        match self {
            Self::Ass(v) => v,
            _ => unreachable!(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LexExprToken {
    pub start: ParserPosition,
    pub len: usize,
    pub r#type: LexExprTokenType,
    pub data: LexExprTokenData,
}

impl LexExprToken {
    pub const fn new() -> Self {
        Self {
            start: ParserPosition::new(),
            len: 0,
            r#type: 0,
            data: LexExprTokenData::new(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeReg {
    pub name: c_int,
}

impl ExprASTNodeReg {
    pub const fn new() -> Self {
        Self { name: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeFigTypeGuesses {
    pub allow_dict: bool,
    pub allow_lambda: bool,
    pub allow_ident: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeFig {
    pub type_guesses: ExprASTNodeFigTypeGuesses,
    pub opening_hl_idx: usize,
}

impl ExprASTNodeFig {
    pub const fn new() -> Self {
        Self {
            type_guesses: ExprASTNodeFigTypeGuesses {
                allow_dict: false,
                allow_lambda: false,
                allow_ident: false,
            },
            opening_hl_idx: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeVar {
    pub scope: ExprVarScope,
    pub ident: *const c_char,
    pub ident_len: usize,
}

impl ExprASTNodeVar {
    pub const fn new() -> Self {
        Self {
            scope: K_EXPR_VAR_SCOPE_MISSING,
            ident: std::ptr::null(),
            ident_len: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeTer {
    pub got_colon: bool,
}

impl ExprASTNodeTer {
    pub const fn new() -> Self {
        Self { got_colon: false }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeCmp {
    pub r#type: ExprComparisonType,
    pub ccs: ExprCaseCompareStrategy,
    pub inv: bool,
}

impl ExprASTNodeCmp {
    pub const fn new() -> Self {
        Self {
            r#type: K_EXPR_CMP_EQUAL,
            ccs: K_CCSTRATEGY_USE_OPTION,
            inv: false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeNum {
    pub value: Uvarnumber,
}

impl ExprASTNodeNum {
    pub const fn new() -> Self {
        Self { value: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeFlt {
    pub value: TvFloat,
}

impl ExprASTNodeFlt {
    pub const fn new() -> Self {
        Self { value: 0.0 }
    }
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct ExprASTNodeStr {
    pub value: Vec<c_char>,
    pub size: usize,
}

impl ExprASTNodeStr {
    pub const fn new() -> Self {
        Self {
            value: Vec::new(),
            size: 0,
        }
    }

    #[inline]
    pub fn value_ptr(&self) -> *const c_char {
        if self.value.is_empty() {
            std::ptr::null()
        } else {
            self.value.as_ptr()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeOpt {
    pub ident: *const c_char,
    pub ident_len: usize,
    pub scope: ExprOptScope,
}

impl ExprASTNodeOpt {
    pub const fn new() -> Self {
        Self {
            ident: std::ptr::null(),
            ident_len: 0,
            scope: K_EXPR_OPT_SCOPE_UNSPECIFIED,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeEnv {
    pub ident: *const c_char,
    pub ident_len: usize,
}

impl ExprASTNodeEnv {
    pub const fn new() -> Self {
        Self {
            ident: std::ptr::null(),
            ident_len: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeAss {
    pub r#type: ExprAssignmentType,
}

impl ExprASTNodeAss {
    pub const fn new() -> Self {
        Self {
            r#type: K_EXPR_ASGN_PLAIN,
        }
    }
}

#[derive(Clone, Default)]
pub enum ExprASTNodeData {
    #[default]
    None,
    Reg(ExprASTNodeReg),
    Fig(ExprASTNodeFig),
    Var(ExprASTNodeVar),
    Ter(ExprASTNodeTer),
    Cmp(ExprASTNodeCmp),
    Num(ExprASTNodeNum),
    Flt(ExprASTNodeFlt),
    Str(ExprASTNodeStr),
    Opt(ExprASTNodeOpt),
    Env(ExprASTNodeEnv),
    Ass(ExprASTNodeAss),
}

impl ExprASTNodeData {
    #[inline]
    pub fn reg(&self) -> &ExprASTNodeReg {
        static EMPTY: ExprASTNodeReg = ExprASTNodeReg::new();
        match self {
            Self::Reg(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn reg_mut(&mut self) -> &mut ExprASTNodeReg {
        if !matches!(self, Self::Reg(_)) {
            *self = Self::Reg(ExprASTNodeReg::new());
        }
        match self {
            Self::Reg(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn fig(&self) -> &ExprASTNodeFig {
        static EMPTY: ExprASTNodeFig = ExprASTNodeFig::new();
        match self {
            Self::Fig(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn fig_mut(&mut self) -> &mut ExprASTNodeFig {
        if !matches!(self, Self::Fig(_)) {
            *self = Self::Fig(ExprASTNodeFig::new());
        }
        match self {
            Self::Fig(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn var(&self) -> &ExprASTNodeVar {
        match self {
            Self::Var(v) => v,
            _ => &const { ExprASTNodeVar::new() },
        }
    }

    #[inline]
    pub fn var_mut(&mut self) -> &mut ExprASTNodeVar {
        if !matches!(self, Self::Var(_)) {
            *self = Self::Var(ExprASTNodeVar::new());
        }
        match self {
            Self::Var(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn ter(&self) -> &ExprASTNodeTer {
        static EMPTY: ExprASTNodeTer = ExprASTNodeTer::new();
        match self {
            Self::Ter(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn ter_mut(&mut self) -> &mut ExprASTNodeTer {
        if !matches!(self, Self::Ter(_)) {
            *self = Self::Ter(ExprASTNodeTer::new());
        }
        match self {
            Self::Ter(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn cmp(&self) -> &ExprASTNodeCmp {
        static EMPTY: ExprASTNodeCmp = ExprASTNodeCmp::new();
        match self {
            Self::Cmp(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn cmp_mut(&mut self) -> &mut ExprASTNodeCmp {
        if !matches!(self, Self::Cmp(_)) {
            *self = Self::Cmp(ExprASTNodeCmp::new());
        }
        match self {
            Self::Cmp(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn num(&self) -> &ExprASTNodeNum {
        static EMPTY: ExprASTNodeNum = ExprASTNodeNum::new();
        match self {
            Self::Num(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn num_mut(&mut self) -> &mut ExprASTNodeNum {
        if !matches!(self, Self::Num(_)) {
            *self = Self::Num(ExprASTNodeNum::new());
        }
        match self {
            Self::Num(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn flt(&self) -> &ExprASTNodeFlt {
        static EMPTY: ExprASTNodeFlt = ExprASTNodeFlt::new();
        match self {
            Self::Flt(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn flt_mut(&mut self) -> &mut ExprASTNodeFlt {
        if !matches!(self, Self::Flt(_)) {
            *self = Self::Flt(ExprASTNodeFlt::new());
        }
        match self {
            Self::Flt(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn str(&self) -> &ExprASTNodeStr {
        static EMPTY: ExprASTNodeStr = ExprASTNodeStr::new();
        match self {
            Self::Str(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn str_mut(&mut self) -> &mut ExprASTNodeStr {
        if !matches!(self, Self::Str(_)) {
            *self = Self::Str(ExprASTNodeStr::new());
        }
        match self {
            Self::Str(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn opt(&self) -> &ExprASTNodeOpt {
        match self {
            Self::Opt(v) => v,
            _ => &const { ExprASTNodeOpt::new() },
        }
    }

    #[inline]
    pub fn opt_mut(&mut self) -> &mut ExprASTNodeOpt {
        if !matches!(self, Self::Opt(_)) {
            *self = Self::Opt(ExprASTNodeOpt::new());
        }
        match self {
            Self::Opt(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn env(&self) -> &ExprASTNodeEnv {
        match self {
            Self::Env(v) => v,
            _ => &const { ExprASTNodeEnv::new() },
        }
    }

    #[inline]
    pub fn env_mut(&mut self) -> &mut ExprASTNodeEnv {
        if !matches!(self, Self::Env(_)) {
            *self = Self::Env(ExprASTNodeEnv::new());
        }
        match self {
            Self::Env(v) => v,
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn ass(&self) -> &ExprASTNodeAss {
        static EMPTY: ExprASTNodeAss = ExprASTNodeAss::new();
        match self {
            Self::Ass(v) => v,
            _ => &EMPTY,
        }
    }

    #[inline]
    pub fn ass_mut(&mut self) -> &mut ExprASTNodeAss {
        if !matches!(self, Self::Ass(_)) {
            *self = Self::Ass(ExprASTNodeAss::new());
        }
        match self {
            Self::Ass(v) => v,
            _ => unreachable!(),
        }
    }
}

#[repr(C)]
pub struct ExprASTNode {
    pub r#type: ExprASTNodeType,
    pub children: Option<Box<ExprASTNode>>,
    pub next: Option<Box<ExprASTNode>>,
    pub start: ParserPosition,
    pub len: usize,
    pub data: ExprASTNodeData,
}

impl ExprASTNode {
    pub const fn new(node_type: ExprASTNodeType) -> Self {
        Self {
            r#type: node_type,
            children: None,
            next: None,
            start: ParserPosition { line: 0, col: 0 },
            len: 0,
            data: ExprASTNodeData::None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTError {
    pub msg: Option<&'static std::ffi::CStr>,
    pub arg: *const c_char,
    pub arg_len: c_int,
}

#[repr(C)]
pub struct ExprAST {
    pub err: ExprASTError,
    pub root: Option<Box<ExprASTNode>>,
}

impl Drop for ExprAST {
    fn drop(&mut self) {
        let mut stack: Vec<Box<ExprASTNode>> = Vec::new();
        if let Some(root) = self.root.take() {
            stack.push(root);
        }
        while let Some(mut node) = stack.pop() {
            if let Some(child) = node.children.take() {
                stack.push(child);
            }
            if let Some(next) = node.next.take() {
                stack.push(next);
            }
        }
    }
}

type ExprASTStack = Vec<*mut Option<Box<ExprASTNode>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExprASTWantedNode {
    Operator = 0,
    Value = 1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExprASTParseType {
    Expr = 0,
    LambdaArguments = 1,
    Assignment = 2,
    SingleAssignment = 3,
}

type ExprASTParseTypeStack = Vec<ExprASTParseType>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ExprOpLvl {
    Invalid = 0,
    ComplexIdentifier = 1,
    Parens = 2,
    Assignment = 3,
    Arrow = 4,
    Comma = 5,
    Colon = 6,
    TernaryValue = 7,
    Ternary = 8,
    Or = 9,
    And = 10,
    Comparison = 11,
    Addition = 12,
    Multiplication = 13,
    Unary = 14,
    Subscript = 15,
    Value = 16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExprOpAssociativity {
    No,
    Left,
    Right,
}

pub static ELTKN_CMP_TYPE_TAB: [&std::ffi::CStr; 5] = [
    c"Equal",
    c"Matches",
    c"Greater",
    c"GreaterOrEqual",
    c"Identical",
];

pub static EXPR_ASGN_TYPE_TAB: [&std::ffi::CStr; 4] = [c"Plain", c"Add", c"Subtract", c"Concat"];

const fn build_ccs_tab() -> [Option<&'static std::ffi::CStr>; 64] {
    let mut tab = [None; 64];
    tab[K_CCSTRATEGY_USE_OPTION as usize] = Some(c"UseOption");
    tab[K_CCSTRATEGY_MATCH_CASE as usize] = Some(c"MatchCase");
    tab[K_CCSTRATEGY_IGNORE_CASE as usize] = Some(c"IgnoreCase");
    tab
}

pub static CCS_TAB: [Option<&std::ffi::CStr>; 64] = build_ccs_tab();

pub static EAST_NODE_TYPE_TAB: [&std::ffi::CStr; 39] = [
    c"Missing",
    c"OpMissing",
    c"Ternary",
    c"TernaryValue",
    c"Register",
    c"Subscript",
    c"ListLiteral",
    c"UnaryPlus",
    c"BinaryPlus",
    c"Nested",
    c"Call",
    c"PlainIdentifier",
    c"PlainKey",
    c"ComplexIdentifier",
    c"UnknownFigure",
    c"Lambda",
    c"DictLiteral",
    c"CurlyBracesIdentifier",
    c"Comma",
    c"Colon",
    c"Arrow",
    c"Comparison",
    c"Concat",
    c"ConcatOrSubscript",
    c"Integer",
    c"Float",
    c"SingleQuotedString",
    c"DoubleQuotedString",
    c"Or",
    c"And",
    c"UnaryMinus",
    c"BinaryMinus",
    c"Not",
    c"Multiplication",
    c"Division",
    c"Mod",
    c"Option",
    c"Environment",
    c"Assignment",
];

const NODE_PROPS: [(ExprOpLvl, ExprOpAssociativity); 39] = [
    (ExprOpLvl::Invalid, ExprOpAssociativity::No), // K_EXPR_NODE_MISSING
    (ExprOpLvl::Multiplication, ExprOpAssociativity::No), // K_EXPR_NODE_OP_MISSING
    (ExprOpLvl::Ternary, ExprOpAssociativity::Right), // K_EXPR_NODE_TERNARY
    (ExprOpLvl::TernaryValue, ExprOpAssociativity::Right), // K_EXPR_NODE_TERNARY_VALUE
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // K_EXPR_NODE_REGISTER
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // K_EXPR_NODE_SUBSCRIPT
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // K_EXPR_NODE_LIST_LITERAL
    (ExprOpLvl::Unary, ExprOpAssociativity::No),   // K_EXPR_NODE_UNARY_PLUS
    (ExprOpLvl::Addition, ExprOpAssociativity::Left), // K_EXPR_NODE_BINARY_PLUS
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // K_EXPR_NODE_NESTED
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // K_EXPR_NODE_CALL
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // K_EXPR_NODE_PLAIN_IDENTIFIER
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // K_EXPR_NODE_PLAIN_KEY
    (ExprOpLvl::Value, ExprOpAssociativity::Left), // K_EXPR_NODE_COMPLEX_IDENTIFIER
    (ExprOpLvl::Parens, ExprOpAssociativity::Left), // K_EXPR_NODE_UNKNOWN_FIGURE
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // K_EXPR_NODE_LAMBDA
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // K_EXPR_NODE_DICT_LITERAL
    (ExprOpLvl::ComplexIdentifier, ExprOpAssociativity::Left), // K_EXPR_NODE_CURLY_BRACES_IDENTIFIER
    (ExprOpLvl::Comma, ExprOpAssociativity::Right),            // K_EXPR_NODE_COMMA
    (ExprOpLvl::Colon, ExprOpAssociativity::No),               // K_EXPR_NODE_COLON
    (ExprOpLvl::Arrow, ExprOpAssociativity::No),               // K_EXPR_NODE_ARROW
    (ExprOpLvl::Comparison, ExprOpAssociativity::Right),       // K_EXPR_NODE_COMPARISON
    (ExprOpLvl::Addition, ExprOpAssociativity::Left),          // K_EXPR_NODE_CONCAT
    (ExprOpLvl::Subscript, ExprOpAssociativity::Left),         // K_EXPR_NODE_CONCAT_OR_SUBSCRIPT
    (ExprOpLvl::Value, ExprOpAssociativity::No),               // K_EXPR_NODE_INTEGER
    (ExprOpLvl::Value, ExprOpAssociativity::No),               // K_EXPR_NODE_FLOAT
    (ExprOpLvl::Value, ExprOpAssociativity::No),               // K_EXPR_NODE_SINGLE_QUOTED_STRING
    (ExprOpLvl::Value, ExprOpAssociativity::No),               // K_EXPR_NODE_DOUBLE_QUOTED_STRING
    (ExprOpLvl::Or, ExprOpAssociativity::Left),                // K_EXPR_NODE_OR
    (ExprOpLvl::And, ExprOpAssociativity::Left),               // K_EXPR_NODE_AND
    (ExprOpLvl::Unary, ExprOpAssociativity::No),               // K_EXPR_NODE_UNARY_MINUS
    (ExprOpLvl::Addition, ExprOpAssociativity::Left),          // K_EXPR_NODE_BINARY_MINUS
    (ExprOpLvl::Unary, ExprOpAssociativity::No),               // K_EXPR_NODE_NOT
    (ExprOpLvl::Multiplication, ExprOpAssociativity::Left),    // K_EXPR_NODE_MULTIPLICATION
    (ExprOpLvl::Multiplication, ExprOpAssociativity::Left),    // K_EXPR_NODE_DIVISION
    (ExprOpLvl::Multiplication, ExprOpAssociativity::Left),    // K_EXPR_NODE_MOD
    (ExprOpLvl::Value, ExprOpAssociativity::No),               // K_EXPR_NODE_OPTION
    (ExprOpLvl::Value, ExprOpAssociativity::No),               // K_EXPR_NODE_ENVIRONMENT
    (ExprOpLvl::Assignment, ExprOpAssociativity::Left),        // K_EXPR_NODE_ASSIGNMENT
];

use crate::charset::{ascii_isident, hex2nr};

#[inline]
fn is_var_scope(b: u8) -> bool {
    matches!(b, b's' | b'g' | b'v' | b'b' | b'w' | b't' | b'l' | b'a')
}

#[inline]
fn is_opt_scope(b: u8) -> bool {
    matches!(b, b'g' | b'l')
}

#[inline]
fn viml_parser_advance(pstate: &mut ParserState<'_>, len: usize) {
    debug_assert_eq!(pstate.pos.line, pstate.reader.lines.len() - 1);
    let pline = *pstate.reader.lines.last().unwrap();
    if pstate.pos.col + len >= pline.size {
        pstate.pos.line += 1;
        pstate.pos.col = 0;
    } else {
        pstate.pos.col += len;
    }
}

#[inline]
fn viml_parser_highlight(
    pstate: &mut ParserState<'_>,
    start: ParserPosition,
    len: usize,
    group: &'static std::ffi::CStr,
) {
    if len == 0 {
        return;
    }
    let Some(colors) = &mut pstate.colors else {
        return;
    };
    debug_assert!(
        colors.is_empty()
            || colors.last().unwrap().start.line < start.line
            || colors.last().unwrap().end_col <= start.col
    );
    colors.push(ParserHighlightChunk {
        start,
        end_col: start.col + len,
        group,
    });
}

#[inline]
fn scale_number(num: TvFloat, base: u8, exponent: Uvarnumber, exponent_negative: bool) -> TvFloat {
    if num == 0.0 || exponent == 0 {
        return num;
    }
    debug_assert_ne!(base, 0);
    let mut exp = exponent;
    let mut p_base = TvFloat::from(base);
    let mut ret = num;
    while exp != 0 {
        if (exp & 1) != 0 {
            if exponent_negative {
                ret /= p_base;
            } else {
                ret *= p_base;
            }
        }
        exp >>= 1;
        p_base *= p_base;
    }
    ret
}

#[inline]
fn get_ccs(ret: &mut LexExprToken, bytes: &[u8]) {
    if ret.len < bytes.len() && (bytes[ret.len] == b'?' || bytes[ret.len] == b'#') {
        if ret.r#type == K_EXPR_LEX_COMPARISON {
            ret.data.cmp_mut().ccs = c_int::from(bytes[ret.len]);
        }
        ret.len += 1;
    } else if ret.r#type == K_EXPR_LEX_COMPARISON {
        ret.data.cmp_mut().ccs = K_CCSTRATEGY_USE_OPTION;
    }
}

/// Get next token for the Vimscript expression input (`viml_pexpr_next_token`).
pub fn viml_pexpr_next_token(pstate: &mut ParserState<'_>, flags: c_int) -> LexExprToken {
    unsafe {
        let mut ret = LexExprToken::new();
        ret.r#type = K_EXPR_LEX_INVALID;
        ret.start = pstate.pos;

        let mut pline = ParserLine {
            data: std::ptr::null(),
            size: 0,
        };
        if !viml_parser_get_remaining_line(pstate, &mut pline) {
            ret.r#type = K_EXPR_LEX_EOC;
            return ret;
        }
        if pline.size == 0 {
            ret.len = 0;
            ret.r#type = K_EXPR_LEX_EOC;
            if (flags & K_ELFLAG_PEEK) == 0 {
                viml_parser_advance(pstate, ret.len);
            }
            return ret;
        }
        let bytes = crate::memory::cstr_slice(pline.data, pline.size);
        ret.len = 1;
        let schar = bytes[0];

        match schar {
            b'(' | b')' => {
                ret.r#type = K_EXPR_LEX_PARENTHESIS;
                ret.data.brc_mut().closing = schar == b')';
            }
            b'[' | b']' => {
                ret.r#type = K_EXPR_LEX_BRACKET;
                ret.data.brc_mut().closing = schar == b']';
            }
            b'{' | b'}' => {
                ret.r#type = K_EXPR_LEX_FIGURE_BRACE;
                ret.data.brc_mut().closing = schar == b'}';
            }
            b'?' => {
                ret.r#type = K_EXPR_LEX_QUESTION;
            }
            b':' => {
                ret.r#type = K_EXPR_LEX_COLON;
            }
            b',' => {
                ret.r#type = K_EXPR_LEX_COMMA;
            }
            b'*' => {
                ret.r#type = K_EXPR_LEX_MULTIPLICATION;
                ret.data.mul_mut().r#type = K_EXPR_LEX_MUL_MUL;
            }
            b'/' => {
                ret.r#type = K_EXPR_LEX_MULTIPLICATION;
                ret.data.mul_mut().r#type = K_EXPR_LEX_MUL_DIV;
            }
            b'%' => {
                ret.r#type = K_EXPR_LEX_MULTIPLICATION;
                ret.data.mul_mut().r#type = K_EXPR_LEX_MUL_MOD;
            }
            b' ' | TAB => {
                ret.r#type = K_EXPR_LEX_SPACING;
                while ret.len < bytes.len() && ascii_iswhite(bytes[ret.len]) {
                    ret.len += 1;
                }
            }
            CTRL_A_CONST..=CTRL_H_CONST | CTRL_K_CONST_CONST..=CTRL_Z_CONST_CONST => {
                ret.r#type = K_EXPR_LEX_INVALID;
                while ret.len < bytes.len() && (bytes[ret.len] as i8) < (b' ' as i8) {
                    ret.len += 1;
                }
                ret.data.err_mut().r#type = K_EXPR_LEX_SPACING;
                ret.data.err_mut().msg =
                    Some(c"E15: Invalid control character present in input: %.*s");
            }
            b'0'..=b'9' => {
                ret.data.num_mut().is_float = false;
                ret.data.num_mut().base = 10;
                let mut frac_start = 0usize;
                let mut exp_start = 0usize;
                let mut frac_end = 0usize;
                let mut exp_negative = false;
                ret.r#type = K_EXPR_LEX_NUMBER;
                while ret.len < bytes.len() && ascii_isdigit(bytes[ret.len]) {
                    ret.len += 1;
                }
                if (flags & K_ELFLAG_ALLOW_FLOAT) != 0 {
                    let non_float_ret = ret;
                    if bytes.len() > ret.len + 1
                        && bytes[ret.len] == b'.'
                        && ascii_isdigit(bytes[ret.len + 1])
                    {
                        ret.len += 1;
                        frac_start = ret.len;
                        frac_end = ret.len;
                        ret.data.num_mut().is_float = true;
                        while ret.len < bytes.len() && ascii_isdigit(bytes[ret.len]) {
                            if bytes[ret.len] != b'0' {
                                frac_end = ret.len + 1;
                            }
                            ret.len += 1;
                        }
                        if bytes.len() > ret.len + 1
                            && (bytes[ret.len] == b'e' || bytes[ret.len] == b'E')
                            && ((bytes.len() > ret.len + 2
                                && (bytes[ret.len + 1] == b'+' || bytes[ret.len + 1] == b'-')
                                && ascii_isdigit(bytes[ret.len + 2]))
                                || ascii_isdigit(bytes[ret.len + 1]))
                        {
                            ret.len += 1;
                            if bytes[ret.len] == b'+' {
                                ret.len += 1;
                            } else if bytes[ret.len] == b'-' {
                                exp_negative = true;
                                ret.len += 1;
                            }
                            exp_start = ret.len;
                            ret.r#type = K_EXPR_LEX_NUMBER;
                            while ret.len < bytes.len() && ascii_isdigit(bytes[ret.len]) {
                                ret.len += 1;
                            }
                        }
                    }
                    if bytes.len() > ret.len
                        && (bytes[ret.len] == b'.' || ascii_isalpha(bytes[ret.len]))
                    {
                        ret = non_float_ret;
                    }
                }
                if ret.data.num().is_float {
                    let mut significand_part: TvFloat = 0.0;
                    let mut exp_part: Uvarnumber = 0;
                    let frac_size = (frac_end - frac_start) as Uvarnumber;
                    for (i, &b) in bytes[..frac_end].iter().enumerate() {
                        if i == frac_start - 1 {
                            continue;
                        }
                        significand_part = significand_part * 10.0 + TvFloat::from(b - b'0');
                    }
                    if exp_start != 0 {
                        vim_str2nr(
                            pline.data.add(exp_start),
                            None,
                            None,
                            0,
                            None,
                            Some(&mut exp_part),
                            (ret.len - exp_start) as c_int,
                            false,
                            None,
                        );
                    }
                    if exp_negative {
                        exp_part += frac_size;
                    } else if exp_part < frac_size {
                        exp_negative = true;
                        exp_part = frac_size - exp_part;
                    } else {
                        exp_part -= frac_size;
                    }
                    ret.data.num_mut().val = LexExprTokenNumVal::Floating(scale_number(
                        significand_part,
                        10,
                        exp_part,
                        exp_negative,
                    ));
                } else {
                    let mut len: c_int = 0;
                    let mut prep: c_int = 0;
                    let mut int_val: Uvarnumber = 0;
                    vim_str2nr(
                        pline.data,
                        Some(&mut prep),
                        Some(&mut len),
                        STR2NR_ALL,
                        None,
                        Some(&mut int_val),
                        pline.size as c_int,
                        false,
                        None,
                    );
                    ret.data.num_mut().val = LexExprTokenNumVal::Integer(int_val);
                    ret.len = len as usize;
                    ret.data.num_mut().base = match prep as u8 {
                        0 => 10,
                        b'0' => 8,
                        b'x' | b'X' => 16,
                        b'b' | b'B' => 2,
                        _ => 10,
                    };
                }
            }
            b'$' => {
                ret.r#type = K_EXPR_LEX_ENV;
                while ret.len < bytes.len() && ascii_isident(bytes[ret.len]) {
                    ret.len += 1;
                }
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                ret.data.var_mut().scope = 0;
                ret.data.var_mut().autoload = false;
                ret.r#type = K_EXPR_LEX_PLAIN_IDENTIFIER;
                while ret.len < bytes.len() && ascii_isident(bytes[ret.len]) {
                    ret.len += 1;
                }
                if (flags & K_ELFLAG_IS_NOT_CMP) == 0
                    && ((ret.len == 2 && &bytes[..2] == b"is")
                        || (ret.len == 5 && &bytes[..5] == b"isnot"))
                {
                    ret.r#type = K_EXPR_LEX_COMPARISON;
                    ret.data.cmp_mut().r#type = K_EXPR_CMP_IDENTICAL;
                    ret.data.cmp_mut().inv = ret.len == 5;
                    get_ccs(&mut ret, bytes);
                } else if ret.len == 1
                    && bytes.len() > 1
                    && is_var_scope(schar)
                    && bytes[ret.len] == b':'
                    && (flags & K_ELFLAG_FORBID_SCOPE) == 0
                {
                    ret.len += 1;
                    ret.data.var_mut().scope = c_int::from(schar);
                    ret.r#type = K_EXPR_LEX_PLAIN_IDENTIFIER;
                    while ret.len < bytes.len()
                        && (ascii_isident(bytes[ret.len]) || bytes[ret.len] == AUTOLOAD_CHAR)
                    {
                        ret.len += 1;
                    }
                    ret.data.var_mut().autoload = bytes[2..ret.len].contains(&AUTOLOAD_CHAR);
                } else if bytes.len() > ret.len && bytes[ret.len] == AUTOLOAD_CHAR {
                    ret.data.var_mut().autoload = true;
                    ret.r#type = K_EXPR_LEX_PLAIN_IDENTIFIER;
                    while ret.len < bytes.len()
                        && (ascii_isident(bytes[ret.len]) || bytes[ret.len] == AUTOLOAD_CHAR)
                    {
                        ret.len += 1;
                    }
                }
            }
            b'&' => {
                if bytes.len() > 1 && bytes[1] == b'&' {
                    ret.r#type = K_EXPR_LEX_AND;
                    ret.len += 1;
                } else if bytes.len() == 1 || !ascii_isalpha(bytes[1]) {
                    ret.r#type = K_EXPR_LEX_INVALID;
                    ret.data.err_mut().r#type = K_EXPR_LEX_OPTION;
                    ret.data.err_mut().msg = Some(c"E112: Option name missing: %.*s");
                } else {
                    ret.r#type = K_EXPR_LEX_OPTION;
                    let p_offset: usize =
                        if bytes.len() > 2 && bytes[2] == b':' && is_opt_scope(bytes[1]) {
                            ret.len += 2;
                            ret.data.opt_mut().scope = c_int::from(bytes[1]);
                            ret.data.opt_mut().name = pline.data.add(3);
                            3
                        } else {
                            ret.data.opt_mut().scope = K_EXPR_OPT_SCOPE_UNSPECIFIED;
                            ret.data.opt_mut().name = pline.data.add(1);
                            1
                        };
                    if bytes.len() - p_offset >= 4
                        && bytes[p_offset] == b't'
                        && bytes[p_offset + 1] == b'_'
                    {
                        ret.data.opt_mut().len = 4;
                        ret.len += 4;
                    } else {
                        let mut p_idx = p_offset;
                        while p_idx < bytes.len() && ascii_isalpha(bytes[p_idx]) {
                            p_idx += 1;
                        }
                        ret.data.opt_mut().len = p_idx - p_offset;
                        if ret.data.opt().len == 0 {
                            ret.r#type = K_EXPR_LEX_INVALID;
                            ret.data.err_mut().r#type = K_EXPR_LEX_OPTION;
                            ret.data.err_mut().msg = Some(c"E112: Option name missing: %.*s");
                        } else {
                            ret.len += ret.data.opt().len;
                        }
                    }
                }
            }
            b'@' => {
                ret.r#type = K_EXPR_LEX_REGISTER;
                if bytes.len() > 1 {
                    ret.len += 1;
                    ret.data.reg_mut().name = c_int::from(bytes[1]);
                } else {
                    ret.data.reg_mut().name = -1;
                }
            }
            b'\'' => {
                ret.r#type = K_EXPR_LEX_SINGLE_QUOTED_STRING;
                ret.data.str_mut().closed = false;
                while ret.len < bytes.len() && !ret.data.str().closed {
                    if bytes[ret.len] == b'\'' {
                        if ret.len + 1 < bytes.len() && bytes[ret.len + 1] == b'\'' {
                            ret.len += 1;
                        } else {
                            ret.data.str_mut().closed = true;
                        }
                    }
                    ret.len += 1;
                }
            }
            b'"' => {
                ret.r#type = K_EXPR_LEX_DOUBLE_QUOTED_STRING;
                ret.data.str_mut().closed = false;
                while ret.len < bytes.len() && !ret.data.str().closed {
                    if bytes[ret.len] == b'\\' {
                        if ret.len + 1 < bytes.len() {
                            ret.len += 1;
                        }
                    } else if bytes[ret.len] == b'"' {
                        ret.data.str_mut().closed = true;
                    }
                    ret.len += 1;
                }
            }
            b'!' | b'=' => {
                if bytes.len() == 1 {
                    ret.r#type = if schar == b'!' {
                        K_EXPR_LEX_NOT
                    } else {
                        K_EXPR_LEX_ASSIGNMENT
                    };
                    ret.data.ass_mut().r#type = K_EXPR_ASGN_PLAIN;
                } else {
                    ret.r#type = K_EXPR_LEX_COMPARISON;
                    ret.data.cmp_mut().inv = schar == b'!';
                    if bytes[1] == b'=' {
                        ret.data.cmp_mut().r#type = K_EXPR_CMP_EQUAL;
                        ret.len += 1;
                    } else if bytes[1] == b'~' {
                        ret.data.cmp_mut().r#type = K_EXPR_CMP_MATCHES;
                        ret.len += 1;
                    } else if schar == b'!' {
                        ret.r#type = K_EXPR_LEX_NOT;
                    } else {
                        ret.r#type = K_EXPR_LEX_ASSIGNMENT;
                        ret.data.ass_mut().r#type = K_EXPR_ASGN_PLAIN;
                    }
                    get_ccs(&mut ret, bytes);
                }
            }
            b'>' | b'<' => {
                ret.r#type = K_EXPR_LEX_COMPARISON;
                let haseqsign = bytes.len() > 1 && bytes[1] == b'=';
                if haseqsign {
                    ret.len += 1;
                }
                get_ccs(&mut ret, bytes);
                ret.data.cmp_mut().inv = schar == b'<';
                ret.data.cmp_mut().r#type = if ret.data.cmp().inv ^ haseqsign {
                    K_EXPR_CMP_GREATER_OR_EQUAL
                } else {
                    K_EXPR_CMP_GREATER
                };
            }
            b'-' => {
                if bytes.len() > 1 && bytes[1] == b'>' {
                    ret.len += 1;
                    ret.r#type = K_EXPR_LEX_ARROW;
                } else if bytes.len() > 1 && bytes[1] == b'=' {
                    ret.len += 1;
                    ret.r#type = K_EXPR_LEX_ASSIGNMENT;
                    ret.data.ass_mut().r#type = K_EXPR_ASGN_SUBTRACT;
                } else {
                    ret.r#type = K_EXPR_LEX_MINUS;
                }
            }
            b'+' => {
                if bytes.len() > 1 && bytes[1] == b'=' {
                    ret.len += 1;
                    ret.r#type = K_EXPR_LEX_ASSIGNMENT;
                    ret.data.ass_mut().r#type = K_EXPR_ASGN_ADD;
                } else {
                    ret.r#type = K_EXPR_LEX_PLUS;
                }
            }
            b'.' => {
                if bytes.len() > 1 && bytes[1] == b'=' {
                    ret.len += 1;
                    ret.r#type = K_EXPR_LEX_ASSIGNMENT;
                    ret.data.ass_mut().r#type = K_EXPR_ASGN_CONCAT;
                } else {
                    ret.r#type = K_EXPR_LEX_DOT;
                }
            }
            NUL | NL => {
                if (flags & K_ELFLAG_FORBID_EOC) != 0 {
                    ret.r#type = K_EXPR_LEX_INVALID;
                    ret.data.err_mut().msg = Some(c"E15: Unexpected EOC character: %.*s");
                    ret.data.err_mut().r#type = K_EXPR_LEX_SPACING;
                } else {
                    ret.r#type = K_EXPR_LEX_EOC;
                }
            }
            b'|' => {
                if bytes.len() >= 2 && bytes[ret.len] == b'|' {
                    ret.len += 1;
                    ret.r#type = K_EXPR_LEX_OR;
                } else if (flags & K_ELFLAG_FORBID_EOC) != 0 {
                    ret.r#type = K_EXPR_LEX_INVALID;
                    ret.data.err_mut().msg = Some(c"E15: Unexpected EOC character: %.*s");
                    ret.data.err_mut().r#type = K_EXPR_LEX_OR;
                } else {
                    ret.r#type = K_EXPR_LEX_EOC;
                }
            }
            _ => {
                ret.len = utfc_ptr2len_len(pline.data, pline.size as c_int) as usize;
                ret.r#type = K_EXPR_LEX_INVALID;
                ret.data.err_mut().r#type = K_EXPR_LEX_PLAIN_IDENTIFIER;
                ret.data.err_mut().msg = Some(c"E15: Unidentified character: %.*s");
            }
        }

        if (flags & K_ELFLAG_PEEK) == 0 {
            viml_parser_advance(pstate, ret.len);
        }
        ret
    }
}

#[inline]
fn viml_pexpr_new_node(node_type: ExprASTNodeType) -> Box<ExprASTNode> {
    Box::new(ExprASTNode::new(node_type))
}

#[inline]
fn new_node_with_cur_pos(
    node_type: ExprASTNodeType,
    cur_token: LexExprToken,
    prev_token: LexExprToken,
) -> Box<ExprASTNode> {
    let mut node = viml_pexpr_new_node(node_type);
    node.start = cur_token.start;
    node.len = cur_token.len;
    if prev_token.r#type == K_EXPR_LEX_SPACING {
        node.start = prev_token.start;
        node.len += prev_token.len;
    }
    node
}

#[inline]
fn node_lvl(node: &ExprASTNode) -> ExprOpLvl {
    NODE_PROPS[node.r#type as usize].0
}

#[inline]
fn node_ass(node: &ExprASTNode) -> ExprOpAssociativity {
    NODE_PROPS[node.r#type as usize].1
}

#[inline]
const fn shifted_pos(pos: ParserPosition, shift: usize) -> ParserPosition {
    ParserPosition {
        line: pos.line,
        col: pos.col + shift,
    }
}

#[inline]
const fn recol_pos(pos: ParserPosition, new_col: usize) -> ParserPosition {
    ParserPosition {
        line: pos.line,
        col: new_col,
    }
}

#[inline]
fn east_set_error(
    pstate: &ParserState<'_>,
    ret_ast_err: &mut ExprASTError,
    msg: &'static std::ffi::CStr,
    start: ParserPosition,
) {
    unsafe {
        if ret_ast_err.msg.is_some() {
            return;
        }
        let pline = pstate.reader.lines.as_slice()[start.line];
        ret_ast_err.msg = Some(msg);
        ret_ast_err.arg_len = (pline.size - start.col) as c_int;
        ret_ast_err.arg = if pline.data.is_null() {
            std::ptr::null()
        } else {
            pline.data.add(start.col)
        };
    }
}

fn viml_pexpr_handle_bop(
    pstate: &ParserState<'_>,
    ast_stack: &mut ExprASTStack,
    mut bop_node: Box<ExprASTNode>,
    want_node_p: &mut ExprASTWantedNode,
    ast_err: &mut ExprASTError,
) -> bool {
    unsafe {
        let mut ret = true;
        let mut top_node_p: *mut Option<Box<ExprASTNode>> = std::ptr::null_mut();
        let mut top_node_lvl = ExprOpLvl::Invalid;
        let mut top_node_ass = ExprOpAssociativity::No;
        debug_assert!(!ast_stack.is_empty());
        let bop_node_lvl =
            if bop_node.r#type == K_EXPR_NODE_CALL || bop_node.r#type == K_EXPR_NODE_SUBSCRIPT {
                ExprOpLvl::Subscript
            } else {
                node_lvl(&bop_node)
            };
        loop {
            let new_top_node_p = *ast_stack.last().unwrap();
            let new_top_node = (*new_top_node_p).as_ref().unwrap();
            let new_top_node_lvl = node_lvl(new_top_node);
            let new_top_node_ass = node_ass(new_top_node);
            if !top_node_p.is_null()
                && (bop_node_lvl > new_top_node_lvl
                    || (bop_node_lvl == new_top_node_lvl
                        && new_top_node_ass == ExprOpAssociativity::No))
            {
                break;
            }
            ast_stack.pop();
            top_node_p = new_top_node_p;
            top_node_lvl = new_top_node_lvl;
            top_node_ass = new_top_node_ass;
            if bop_node_lvl == top_node_lvl && top_node_ass == ExprOpAssociativity::Right {
                break;
            }
            if ast_stack.is_empty() {
                break;
            }
        }
        if top_node_ass == ExprOpAssociativity::Left || top_node_lvl != bop_node_lvl {
            bop_node.children = (*top_node_p).take();
            debug_assert!(bop_node.children.as_ref().unwrap().next.is_none());
            *top_node_p = Some(bop_node);
            let bop_ref = (*top_node_p).as_deref_mut().unwrap();
            let child_ref = bop_ref.children.as_deref_mut().unwrap();
            ast_stack.push(top_node_p);
            ast_stack.push(&raw mut child_ref.next);
        } else {
            debug_assert!(
                top_node_lvl == bop_node_lvl && top_node_ass == ExprOpAssociativity::Right
            );
            let top_ref = (*top_node_p).as_deref_mut().unwrap();
            debug_assert!(
                top_ref.children.is_some() && top_ref.children.as_ref().unwrap().next.is_some()
            );
            let top_child = top_ref.children.as_deref_mut().unwrap();
            let bop_type = bop_node.r#type;
            let bop_start = bop_node.start;
            bop_node.children = top_child.next.take();
            debug_assert!(bop_node.children.as_ref().unwrap().next.is_none());
            top_child.next = Some(bop_node);
            let next_slot: *mut Option<Box<ExprASTNode>> = &raw mut top_child.next;
            let bop_ref = (*next_slot).as_deref_mut().unwrap();
            let bop_child = bop_ref.children.as_deref_mut().unwrap();
            ast_stack.push(top_node_p);
            ast_stack.push(next_slot);
            ast_stack.push(&raw mut bop_child.next);
            if bop_type == K_EXPR_NODE_COMPARISON {
                east_set_error(
                    pstate,
                    ast_err,
                    c"E15: Operator is not associative: %.*s",
                    bop_start,
                );
                ret = false;
            }
        }
        *want_node_p = ExprASTWantedNode::Value;
        ret
    }
}

#[inline]
const fn pt_is_assignment(pt: ExprASTParseType) -> bool {
    matches!(
        pt,
        ExprASTParseType::Assignment | ExprASTParseType::SingleAssignment
    )
}

#[repr(C)]
#[derive(Clone, Copy)]
struct StringShift {
    start: usize,
    orig_len: usize,
    act_len: usize,
    escape_not_known: bool,
}

macro_rules! hl_group {
    ($is_invalid:expr, $valid:literal, $invalid:literal) => {
        if $is_invalid { $invalid } else { $valid }
    };
}

fn parse_quoted_string(
    pstate: &mut ParserState<'_>,
    node: &mut ExprASTNode,
    token: LexExprToken,
    _ast_stack: &ExprASTStack,
    is_invalid: bool,
) {
    unsafe {
        let pline = pstate.reader.lines.as_slice()[token.start.line];
        let s = pline.data.add(token.start.col);
        let closed_len = usize::from(token.data.str().closed);
        let e = s.add(token.len - closed_len);
        let mut p = s.add(1);
        let is_double = token.r#type == K_EXPR_LEX_DOUBLE_QUOTED_STRING;
        let mut size = token.len - closed_len - 1;
        let mut shifts: Vec<StringShift> = Vec::new();

        if !is_double {
            viml_parser_highlight(
                pstate,
                token.start,
                1,
                hl_group!(is_invalid, c"NvimSingleQuote", c"NvimInvalidSingleQuote"),
            );
            while (p as usize) < (e as usize) {
                let rem = e as usize - p as usize;
                let slice = crate::memory::cstr_slice(p, rem);
                let Some(pos) = slice.iter().position(|&b| b == b'\'') else {
                    break;
                };
                let chunk_e = p.add(pos);
                size -= 1;
                p = chunk_e.add(2);
                if pstate.colors.is_some() {
                    shifts.push(StringShift {
                        start: token.start.col + (chunk_e as usize - s as usize),
                        orig_len: 2,
                        act_len: 1,
                        escape_not_known: false,
                    });
                }
            }
            node.data.str_mut().size = size;
            if size == 0 {
                node.data.str_mut().value = Vec::new();
            } else {
                let mut buf = vec![0 as c_char; size + 1];
                let mut v_p = buf.as_mut_ptr();
                p = s.add(1);
                while (p as usize) < (e as usize) {
                    let rem = e as usize - p as usize;
                    let slice = crate::memory::cstr_slice(p, rem);
                    let Some(pos) = slice.iter().position(|&b| b == b'\'') else {
                        std::ptr::copy_nonoverlapping(p, v_p, rem);
                        break;
                    };
                    let chunk_e = p.add(pos);
                    std::ptr::copy_nonoverlapping(p, v_p, pos);
                    v_p = v_p.add(pos + 1);
                    *v_p.sub(1) = b'\'' as c_char;
                    p = chunk_e.add(2);
                }
                node.data.str_mut().value = buf;
            }
        } else {
            viml_parser_highlight(
                pstate,
                token.start,
                1,
                hl_group!(is_invalid, c"NvimDoubleQuote", c"NvimInvalidDoubleQuote"),
            );
            p = s.add(1);
            while (p as usize) < (e as usize) {
                if crate::memory::cstr_first_byte(p) == b'\\' && (p.add(1) as usize) < (e as usize)
                {
                    p = p.add(1);
                    if std::ptr::eq(p.add(1), e) {
                        size -= 1;
                        break;
                    }
                    match crate::memory::cstr_first_byte(p) {
                        b'<' => {
                            size += 5;
                        }
                        b'x' | b'X' => {
                            size -= 1;
                            if ascii_isxdigit(crate::memory::cstr_byte_at(p, 1)) {
                                size -= 1;
                                if (p.add(2) as usize) < (e as usize)
                                    && ascii_isxdigit(crate::memory::cstr_byte_at(p, 2))
                                {
                                    size -= 1;
                                }
                            }
                        }
                        b'u' | b'U' => {
                            let esc_start = p;
                            let mut n: usize = if crate::memory::cstr_first_byte(p) == b'u' {
                                4
                            } else {
                                8
                            };
                            let mut nr: c_int = 0;
                            p = p.add(1);
                            while (p.add(1) as usize) < (e as usize)
                                && n != 0
                                && ascii_isxdigit(crate::memory::cstr_byte_at(p, 1))
                            {
                                n -= 1;
                                p = p.add(1);
                                nr = (nr << 4)
                                    + hex2nr(c_int::from(crate::memory::cstr_first_byte(p)));
                            }
                            let esc_len = (p as usize) - (esc_start as usize - 1);
                            let char_len = utf_char2len(nr) as usize;
                            size = size.wrapping_sub(esc_len.wrapping_sub(char_len));
                            p = p.sub(1);
                        }
                        b'0'..=b'7' => {
                            size -= 1;
                            p = p.add(1);
                            if (crate::memory::cstr_first_byte(p)) >= b'0'
                                && (crate::memory::cstr_first_byte(p)) <= b'7'
                            {
                                size -= 1;
                                p = p.add(1);
                                if (p as usize) < (e as usize)
                                    && (crate::memory::cstr_first_byte(p)) >= b'0'
                                    && (crate::memory::cstr_first_byte(p)) <= b'7'
                                {
                                    size -= 1;
                                    p = p.add(1);
                                }
                            }
                        }
                        _ => {
                            size -= 1;
                        }
                    }
                }
                p = p.add(1);
            }
            if size == 0 {
                node.data.str_mut().value = Vec::new();
                node.data.str_mut().size = 0;
            } else {
                let mut buf = vec![0 as c_char; size];
                let mut v_p = buf.as_mut_ptr();
                p = s.add(1);
                while (p as usize) < (e as usize) {
                    let rem = e as usize - p as usize;
                    let slice = crate::memory::cstr_slice(p, rem);
                    let Some(pos) = slice.iter().position(|&b| b == b'\\') else {
                        std::ptr::copy_nonoverlapping(p, v_p, rem);
                        v_p = v_p.add(rem);
                        break;
                    };
                    let chunk_e = p.add(pos);
                    std::ptr::copy_nonoverlapping(p, v_p, pos);
                    v_p = v_p.add(pos);
                    p = chunk_e.add(1);
                    if std::ptr::eq(p, e) {
                        *v_p = b'\\' as c_char;
                        v_p = v_p.add(1);
                        break;
                    }
                    let mut is_unknown = false;
                    let v_p_start = v_p;
                    match crate::memory::cstr_first_byte(p) {
                        b'b' => {
                            *v_p = BS as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'e' => {
                            *v_p = ESC as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'f' => {
                            *v_p = FF as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'n' => {
                            *v_p = NL as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'r' => {
                            *v_p = CAR as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b't' => {
                            *v_p = TAB as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'"' => {
                            *v_p = b'"' as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'\\' => {
                            *v_p = b'\\' as c_char;
                            v_p = v_p.add(1);
                            p = p.add(1);
                        }
                        b'X' | b'x' | b'u' | b'U' => {
                            if (p.add(1) as usize) < (e as usize)
                                && ascii_isxdigit(crate::memory::cstr_byte_at(p, 1))
                            {
                                let is_hex = crate::memory::cstr_first_byte(p) == b'x'
                                    || crate::memory::cstr_first_byte(p) == b'X';
                                let mut n: usize = if is_hex {
                                    2
                                } else if crate::memory::cstr_first_byte(p) == b'u' {
                                    4
                                } else {
                                    8
                                };
                                let mut nr: c_int = 0;
                                while (p.add(1) as usize) < (e as usize)
                                    && n != 0
                                    && ascii_isxdigit(crate::memory::cstr_byte_at(p, 1))
                                {
                                    n -= 1;
                                    p = p.add(1);
                                    nr = (nr << 4)
                                        + hex2nr(c_int::from(crate::memory::cstr_first_byte(p)));
                                }
                                p = p.add(1);
                                if is_hex {
                                    *v_p = (nr as u8) as c_char;
                                    v_p = v_p.add(1);
                                } else {
                                    v_p = v_p.add(utf_char2bytes(nr, v_p) as usize);
                                }
                            } else {
                                is_unknown = true;
                                *v_p = *p;
                                v_p = v_p.add(1);
                                p = p.add(1);
                            }
                        }
                        b'0'..=b'7' => {
                            let mut ch = (crate::memory::cstr_first_byte(p)) - b'0';
                            p = p.add(1);
                            if (p as usize) < (e as usize)
                                && (crate::memory::cstr_first_byte(p)) >= b'0'
                                && (crate::memory::cstr_first_byte(p)) <= b'7'
                            {
                                ch = (ch << 3)
                                    .wrapping_add(crate::memory::cstr_first_byte(p) - b'0');
                                p = p.add(1);
                                if (p as usize) < (e as usize)
                                    && (crate::memory::cstr_first_byte(p)) >= b'0'
                                    && (crate::memory::cstr_first_byte(p)) <= b'7'
                                {
                                    ch = (ch << 3)
                                        .wrapping_add(crate::memory::cstr_first_byte(p) - b'0');
                                    p = p.add(1);
                                }
                            }
                            *v_p = ch as c_char;
                            v_p = v_p.add(1);
                        }
                        b'<' => {
                            let mut flags = FSK_KEYCODE | FSK_IN_STRING;
                            if crate::memory::cstr_byte_at(p, 1) != b'*' {
                                flags |= FSK_SIMPLIFY;
                            }
                            let special_len = trans_special(
                                &mut p,
                                e as usize - p as usize,
                                v_p,
                                flags,
                                false,
                                None,
                            ) as usize;
                            if special_len != 0 {
                                v_p = v_p.add(special_len);
                            } else {
                                is_unknown = true;
                                mb_copy_char(&mut p, &mut v_p);
                            }
                        }
                        _ => {
                            is_unknown = true;
                            mb_copy_char(&mut p, &mut v_p);
                        }
                    }
                    if pstate.colors.is_some() {
                        shifts.push(StringShift {
                            start: token.start.col + (chunk_e as usize - s as usize),
                            orig_len: p as usize - chunk_e as usize,
                            act_len: v_p as usize - v_p_start as usize,
                            escape_not_known: is_unknown,
                        });
                    }
                }
                let actual_len = v_p as usize - buf.as_ptr() as usize;
                buf.truncate(actual_len);
                node.data.str_mut().size = actual_len;
                node.data.str_mut().value = buf;
            }
        }
        if pstate.colors.is_some() {
            let mut next_col = token.start.col + 1;
            let body_str = if is_double {
                hl_group!(
                    is_invalid,
                    c"NvimDoubleQuotedBody",
                    c"NvimInvalidDoubleQuotedBody"
                )
            } else {
                hl_group!(
                    is_invalid,
                    c"NvimSingleQuotedBody",
                    c"NvimInvalidSingleQuotedBody"
                )
            };
            let esc_str = if is_double {
                hl_group!(
                    is_invalid,
                    c"NvimDoubleQuotedEscape",
                    c"NvimInvalidDoubleQuotedEscape"
                )
            } else {
                hl_group!(
                    is_invalid,
                    c"NvimSingleQuotedQuote",
                    c"NvimInvalidSingleQuotedQuote"
                )
            };
            let ukn_esc_str = if is_double {
                hl_group!(
                    is_invalid,
                    c"NvimDoubleQuotedUnknownEscape",
                    c"NvimInvalidDoubleQuotedUnknownEscape"
                )
            } else {
                hl_group!(
                    is_invalid,
                    c"NvimSingleQuotedUnknownEscape",
                    c"NvimInvalidSingleQuotedUnknownEscape"
                )
            };
            for cur_shift in &shifts {
                if cur_shift.start > next_col {
                    viml_parser_highlight(
                        pstate,
                        recol_pos(token.start, next_col),
                        cur_shift.start - next_col,
                        body_str,
                    );
                }
                viml_parser_highlight(
                    pstate,
                    recol_pos(token.start, cur_shift.start),
                    cur_shift.orig_len,
                    if cur_shift.escape_not_known {
                        ukn_esc_str
                    } else {
                        esc_str
                    },
                );
                next_col = cur_shift.start + cur_shift.orig_len;
            }
            if next_col - token.start.col < token.len - closed_len {
                viml_parser_highlight(
                    pstate,
                    recol_pos(token.start, next_col),
                    token.start.col + token.len - closed_len - next_col,
                    body_str,
                );
            }
        }
        if token.data.str().closed {
            if is_double {
                viml_parser_highlight(
                    pstate,
                    shifted_pos(token.start, token.len - 1),
                    1,
                    hl_group!(is_invalid, c"NvimDoubleQuote", c"NvimInvalidDoubleQuote"),
                );
            } else {
                viml_parser_highlight(
                    pstate,
                    shifted_pos(token.start, token.len - 1),
                    1,
                    hl_group!(is_invalid, c"NvimSingleQuote", c"NvimInvalidSingleQuote"),
                );
            }
        }
    }
}

const WANT_NODE_TO_LEXER_FLAGS: [c_int; 2] = [
    K_ELFLAG_FORBID_SCOPE, // Operator
    K_ELFLAG_IS_NOT_CMP,   // Value
];

const BASE_TO_PREFIX_LENGTH: [usize; 17] = [0, 0, 2, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2];

/// Parse one Vimscript expression (`viml_pexpr_parse`).
pub fn viml_pexpr_parse(pstate: &mut ParserState<'_>, flags: c_int) -> ExprAST {
    unsafe {
        let mut ast = ExprAST {
            err: ExprASTError {
                msg: None,
                arg: std::ptr::null(),
                arg_len: 0,
            },
            root: None,
        };
        let mut ast_stack: ExprASTStack = vec![&raw mut ast.root];
        let mut want_node = ExprASTWantedNode::Value;
        let mut pt_stack: ExprASTParseTypeStack = vec![ExprASTParseType::Expr];
        if (flags & K_EXPR_FLAGS_PARSE_LET) != 0 {
            pt_stack.push(ExprASTParseType::Assignment);
        }
        let mut prev_token = LexExprToken::new();
        prev_token.r#type = K_EXPR_LEX_MISSING;
        let mut highlighted_prev_spacing = false;
        let mut lambda_node: *mut ExprASTNode = std::ptr::null_mut();
        let mut asgn_level: usize = 0;

        'parse_loop: loop {
            let is_concat_or_subscript = want_node == ExprASTWantedNode::Value
                && ast_stack.len() > 1
                && (*ast_stack[ast_stack.len() - 2]).as_ref().unwrap().r#type
                    == K_EXPR_NODE_CONCAT_OR_SUBSCRIPT;
            let lexer_additional_flags = K_ELFLAG_PEEK
                | if (flags & K_EXPR_FLAGS_DISALLOW_EOC) != 0 {
                    K_ELFLAG_FORBID_EOC
                } else {
                    0
                }
                | if want_node == ExprASTWantedNode::Value
                    && (ast_stack.len() == 1
                        || ((*ast_stack[ast_stack.len() - 2]).as_ref().unwrap().r#type
                            != K_EXPR_NODE_CONCAT
                            && (*ast_stack[ast_stack.len() - 2]).as_ref().unwrap().r#type
                                != K_EXPR_NODE_CONCAT_OR_SUBSCRIPT))
                {
                    K_ELFLAG_ALLOW_FLOAT
                } else {
                    0
                };
            let mut cur_token = viml_pexpr_next_token(
                pstate,
                WANT_NODE_TO_LEXER_FLAGS[want_node as usize] | lexer_additional_flags,
            );
            if cur_token.r#type == K_EXPR_LEX_EOC {
                break;
            }
            let mut tok_type = cur_token.r#type;
            let mut is_invalid = tok_type == K_EXPR_LEX_INVALID;

            'process_token: loop {
                cur_token = viml_pexpr_next_token(
                    pstate,
                    WANT_NODE_TO_LEXER_FLAGS[want_node as usize] | lexer_additional_flags,
                );
                if tok_type == K_EXPR_LEX_SPACING {
                    if is_invalid {
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimSpacing", c"NvimInvalidSpacing"),
                        );
                    }
                    break 'process_token;
                } else if is_invalid
                    && prev_token.r#type == K_EXPR_LEX_SPACING
                    && !highlighted_prev_spacing
                {
                    viml_parser_highlight(
                        pstate,
                        prev_token.start,
                        prev_token.len,
                        hl_group!(is_invalid, c"NvimSpacing", c"NvimInvalidSpacing"),
                    );
                    is_invalid = false;
                    highlighted_prev_spacing = true;
                }
                let pline = pstate.reader.lines.as_slice()[cur_token.start.line];
                let top_node_p = &mut **ast_stack.last().unwrap();
                debug_assert!(!ast_stack.is_empty());

                let node_is_key = is_concat_or_subscript
                    && (if cur_token.r#type == K_EXPR_LEX_PLAIN_IDENTIFIER {
                        !cur_token.data.var().autoload
                            && cur_token.data.var().scope == K_EXPR_VAR_SCOPE_MISSING
                    } else {
                        cur_token.r#type == K_EXPR_LEX_NUMBER
                    })
                    && prev_token.r#type != K_EXPR_LEX_SPACING;
                if is_concat_or_subscript && !node_is_key {
                    let prev_idx = ast_stack.len() - 2;
                    (*ast_stack[prev_idx]).as_mut().unwrap().r#type = K_EXPR_NODE_CONCAT;
                }

                let is_single_assignment =
                    *pt_stack.last().unwrap() == ExprASTParseType::SingleAssignment;
                match *pt_stack.last().unwrap() {
                    ExprASTParseType::Expr => {}
                    ExprASTParseType::LambdaArguments => {
                        if (want_node == ExprASTWantedNode::Operator
                            && tok_type != K_EXPR_LEX_COMMA
                            && tok_type != K_EXPR_LEX_ARROW)
                            || (want_node == ExprASTWantedNode::Value
                                && !(cur_token.r#type == K_EXPR_LEX_PLAIN_IDENTIFIER
                                    && cur_token.data.var().scope == K_EXPR_VAR_SCOPE_MISSING
                                    && !cur_token.data.var().autoload)
                                && tok_type != K_EXPR_LEX_ARROW)
                        {
                            (*lambda_node).data.fig_mut().type_guesses.allow_lambda = false;
                            if (*lambda_node)
                                .children
                                .as_ref()
                                .is_some_and(|c| c.r#type == K_EXPR_NODE_COMMA)
                            {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Expected lambda arguments list or arrow: %.*s",
                                    cur_token.start,
                                );
                            } else {
                                lambda_node = std::ptr::null_mut();
                                pt_stack.pop();
                            }
                        }
                    }
                    ExprASTParseType::SingleAssignment | ExprASTParseType::Assignment => {
                        if want_node == ExprASTWantedNode::Value
                            && tok_type != K_EXPR_LEX_BRACKET
                            && tok_type != K_EXPR_LEX_PLAIN_IDENTIFIER
                            && (tok_type != K_EXPR_LEX_FIGURE_BRACE || cur_token.data.brc().closing)
                            && !(node_is_key && tok_type == K_EXPR_LEX_NUMBER)
                            && tok_type != K_EXPR_LEX_ENV
                            && tok_type != K_EXPR_LEX_OPTION
                            && tok_type != K_EXPR_LEX_REGISTER
                        {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Expected value part of assignment lvalue: %.*s",
                                cur_token.start,
                            );
                            pt_stack.pop();
                        } else if want_node == ExprASTWantedNode::Operator
                            && tok_type != K_EXPR_LEX_BRACKET
                            && (tok_type != K_EXPR_LEX_FIGURE_BRACE || cur_token.data.brc().closing)
                            && tok_type != K_EXPR_LEX_DOT
                            && (tok_type != K_EXPR_LEX_COMMA || !is_single_assignment)
                            && tok_type != K_EXPR_LEX_ASSIGNMENT
                            && !((tok_type == K_EXPR_LEX_PLAIN_IDENTIFIER
                                || (tok_type == K_EXPR_LEX_FIGURE_BRACE
                                    && !cur_token.data.brc().closing))
                                && prev_token.r#type != K_EXPR_LEX_SPACING)
                        {
                            if (flags & K_EXPR_FLAGS_MULTI) != 0 && ast_stack.len() == 1 {
                                break 'parse_loop;
                            }
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Expected assignment operator or subscript: %.*s",
                                cur_token.start,
                            );
                            pt_stack.pop();
                        }
                        debug_assert!(!pt_stack.is_empty());
                    }
                }
                debug_assert!(!pt_stack.is_empty());
                let cur_pt = *pt_stack.last().unwrap();
                debug_assert!(lambda_node.is_null() || cur_pt == ExprASTParseType::LambdaArguments);

                macro_rules! op_missing {
                    () => {{
                        if (flags & K_EXPR_FLAGS_MULTI) != 0 && ast_stack.len() == 1 {
                            break 'parse_loop;
                        } else {
                            debug_assert!((*top_node_p).is_some());
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Missing operator: %.*s",
                                cur_token.start,
                            );
                            let mut cur_node = new_node_with_cur_pos(
                                K_EXPR_NODE_OP_MISSING,
                                cur_token,
                                prev_token,
                            );
                            cur_node.len = 0;
                            let _ = viml_pexpr_handle_bop(
                                pstate,
                                &mut ast_stack,
                                cur_node,
                                &mut want_node,
                                &mut ast.err,
                            );
                            is_invalid = true;
                            continue 'process_token;
                        }
                    }};
                }

                macro_rules! add_value_if_missing {
                    ($msg:expr) => {{
                        if want_node == ExprASTWantedNode::Value {
                            is_invalid = true;
                            east_set_error(pstate, &mut ast.err, $msg, cur_token.start);
                            let mut cur_node =
                                new_node_with_cur_pos(K_EXPR_NODE_MISSING, cur_token, prev_token);
                            cur_node.len = 0;
                            *top_node_p = Some(cur_node);
                            want_node = ExprASTWantedNode::Operator;
                        }
                    }};
                }

                match tok_type {
                    K_EXPR_LEX_MISSING | K_EXPR_LEX_SPACING | K_EXPR_LEX_EOC => {
                        std::process::abort();
                    }
                    K_EXPR_LEX_INVALID => {
                        is_invalid = true;
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            cur_token.data.err().msg.unwrap(),
                            cur_token.start,
                        );
                        tok_type = cur_token.data.err().r#type;
                        continue 'process_token;
                    }
                    K_EXPR_LEX_REGISTER => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let mut cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_REGISTER, cur_token, prev_token);
                        cur_node.data.reg_mut().name = cur_token.data.reg().name;
                        *top_node_p = Some(cur_node);
                        want_node = ExprASTWantedNode::Operator;
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimRegister", c"NvimInvalidRegister"),
                        );
                    }
                    K_EXPR_LEX_PLUS => {
                        if want_node == ExprASTWantedNode::Value {
                            let cur_node = new_node_with_cur_pos(
                                K_EXPR_NODE_UNARY_PLUS,
                                cur_token,
                                prev_token,
                            );
                            *top_node_p = Some(cur_node);
                            let node_ref = (*top_node_p).as_deref_mut().unwrap();
                            ast_stack.push(&raw mut node_ref.children);
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimUnaryPlus", c"NvimInvalidUnaryPlus"),
                            );
                        } else {
                            let cur_node = new_node_with_cur_pos(
                                K_EXPR_NODE_BINARY_PLUS,
                                cur_token,
                                prev_token,
                            );
                            is_invalid |= !viml_pexpr_handle_bop(
                                pstate,
                                &mut ast_stack,
                                cur_node,
                                &mut want_node,
                                &mut ast.err,
                            );
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimBinaryPlus", c"NvimInvalidBinaryPlus"),
                            );
                        }
                        want_node = ExprASTWantedNode::Value;
                    }
                    K_EXPR_LEX_MINUS => {
                        if want_node == ExprASTWantedNode::Value {
                            let cur_node = new_node_with_cur_pos(
                                K_EXPR_NODE_UNARY_MINUS,
                                cur_token,
                                prev_token,
                            );
                            *top_node_p = Some(cur_node);
                            let node_ref = (*top_node_p).as_deref_mut().unwrap();
                            ast_stack.push(&raw mut node_ref.children);
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimUnaryMinus", c"NvimInvalidUnaryMinus"),
                            );
                        } else {
                            let cur_node = new_node_with_cur_pos(
                                K_EXPR_NODE_BINARY_MINUS,
                                cur_token,
                                prev_token,
                            );
                            is_invalid |= !viml_pexpr_handle_bop(
                                pstate,
                                &mut ast_stack,
                                cur_node,
                                &mut want_node,
                                &mut ast.err,
                            );
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(
                                    is_invalid,
                                    c"NvimBinaryMinus",
                                    c"NvimInvalidBinaryMinus"
                                ),
                            );
                        }
                        want_node = ExprASTWantedNode::Value;
                    }
                    K_EXPR_LEX_OR => {
                        add_value_if_missing!(c"E15: Unexpected or operator: %.*s");
                        let cur_node = new_node_with_cur_pos(K_EXPR_NODE_OR, cur_token, prev_token);
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimOr", c"NvimInvalidOr"),
                        );
                        let _ = viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                    }
                    K_EXPR_LEX_AND => {
                        add_value_if_missing!(c"E15: Unexpected and operator: %.*s");
                        let cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_AND, cur_token, prev_token);
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimAnd", c"NvimInvalidAnd"),
                        );
                        let _ = viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                    }
                    K_EXPR_LEX_MULTIPLICATION => {
                        add_value_if_missing!(
                            c"E15: Unexpected multiplication-like operator: %.*s"
                        );
                        let cur_node = match cur_token.data.mul().r#type {
                            K_EXPR_LEX_MUL_MUL => {
                                let n = new_node_with_cur_pos(
                                    K_EXPR_NODE_MULTIPLICATION,
                                    cur_token,
                                    prev_token,
                                );
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimMultiplication",
                                        c"NvimInvalidMultiplication"
                                    ),
                                );
                                n
                            }
                            K_EXPR_LEX_MUL_DIV => {
                                let n = new_node_with_cur_pos(
                                    K_EXPR_NODE_DIVISION,
                                    cur_token,
                                    prev_token,
                                );
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(is_invalid, c"NvimDivision", c"NvimInvalidDivision"),
                                );
                                n
                            }
                            _ => {
                                let n =
                                    new_node_with_cur_pos(K_EXPR_NODE_MOD, cur_token, prev_token);
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(is_invalid, c"NvimMod", c"NvimInvalidMod"),
                                );
                                n
                            }
                        };
                        let _ = viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                    }
                    K_EXPR_LEX_OPTION => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let mut cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_OPTION, cur_token, prev_token);
                        if cur_token.r#type == K_EXPR_LEX_INVALID {
                            cur_node.data.opt_mut().ident =
                                pline.data.add(cur_token.start.col + cur_token.len);
                            cur_node.data.opt_mut().ident_len = 0;
                            cur_node.data.opt_mut().scope = if cur_token.len == 3 {
                                c_int::from(crate::memory::cstr_byte_at(
                                    pline.data,
                                    cur_token.start.col + 1,
                                ))
                            } else {
                                K_EXPR_OPT_SCOPE_UNSPECIFIED
                            };
                        } else {
                            cur_node.data.opt_mut().ident = cur_token.data.opt().name;
                            cur_node.data.opt_mut().ident_len = cur_token.data.opt().len;
                            cur_node.data.opt_mut().scope = cur_token.data.opt().scope;
                        }
                        *top_node_p = Some(cur_node);
                        want_node = ExprASTWantedNode::Operator;
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            1,
                            hl_group!(is_invalid, c"NvimOptionSigil", c"NvimInvalidOptionSigil"),
                        );
                        let scope_shift =
                            if cur_token.data.opt().scope == K_EXPR_OPT_SCOPE_UNSPECIFIED {
                                0
                            } else {
                                2
                            };
                        if scope_shift != 0 {
                            viml_parser_highlight(
                                pstate,
                                shifted_pos(cur_token.start, 1),
                                1,
                                hl_group!(
                                    is_invalid,
                                    c"NvimOptionScope",
                                    c"NvimInvalidOptionScope"
                                ),
                            );
                            viml_parser_highlight(
                                pstate,
                                shifted_pos(cur_token.start, 2),
                                1,
                                hl_group!(
                                    is_invalid,
                                    c"NvimOptionScopeDelimiter",
                                    c"NvimInvalidOptionScopeDelimiter"
                                ),
                            );
                        }
                        viml_parser_highlight(
                            pstate,
                            shifted_pos(cur_token.start, scope_shift + 1),
                            cur_token.len - (scope_shift + 1),
                            hl_group!(is_invalid, c"NvimOptionName", c"NvimInvalidOptionName"),
                        );
                    }
                    K_EXPR_LEX_ENV => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let mut cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_ENVIRONMENT, cur_token, prev_token);
                        cur_node.data.env_mut().ident = pline.data.add(cur_token.start.col + 1);
                        cur_node.data.env_mut().ident_len = cur_token.len - 1;
                        if cur_node.data.env().ident_len == 0 {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Environment variable name missing",
                                cur_token.start,
                            );
                        }
                        *top_node_p = Some(cur_node);
                        want_node = ExprASTWantedNode::Operator;
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            1,
                            hl_group!(
                                is_invalid,
                                c"NvimEnvironmentSigil",
                                c"NvimInvalidEnvironmentSigil"
                            ),
                        );
                        viml_parser_highlight(
                            pstate,
                            shifted_pos(cur_token.start, 1),
                            cur_token.len - 1,
                            hl_group!(
                                is_invalid,
                                c"NvimEnvironmentName",
                                c"NvimInvalidEnvironmentName"
                            ),
                        );
                    }
                    K_EXPR_LEX_NOT => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_NOT, cur_token, prev_token);
                        *top_node_p = Some(cur_node);
                        let node_ref = (*top_node_p).as_deref_mut().unwrap();
                        ast_stack.push(&raw mut node_ref.children);
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimNot", c"NvimInvalidNot"),
                        );
                    }
                    K_EXPR_LEX_COMPARISON => {
                        add_value_if_missing!(
                            c"E15: Expected value, got comparison operator: %.*s"
                        );
                        let mut cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_COMPARISON, cur_token, prev_token);
                        if cur_token.r#type == K_EXPR_LEX_INVALID {
                            cur_node.data.cmp_mut().ccs = K_CCSTRATEGY_USE_OPTION;
                            cur_node.data.cmp_mut().r#type = K_EXPR_CMP_EQUAL;
                            cur_node.data.cmp_mut().inv = false;
                        } else {
                            cur_node.data.cmp_mut().ccs = cur_token.data.cmp().ccs;
                            cur_node.data.cmp_mut().r#type = cur_token.data.cmp().r#type;
                            cur_node.data.cmp_mut().inv = cur_token.data.cmp().inv;
                        }
                        is_invalid |= !viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                        if cur_token.data.cmp().ccs != K_CCSTRATEGY_USE_OPTION {
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len - 1,
                                hl_group!(is_invalid, c"NvimComparison", c"NvimInvalidComparison"),
                            );
                            viml_parser_highlight(
                                pstate,
                                shifted_pos(cur_token.start, cur_token.len - 1),
                                1,
                                hl_group!(
                                    is_invalid,
                                    c"NvimComparisonModifier",
                                    c"NvimInvalidComparisonModifier"
                                ),
                            );
                        } else {
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimComparison", c"NvimInvalidComparison"),
                            );
                        }
                        want_node = ExprASTWantedNode::Value;
                    }
                    K_EXPR_LEX_COMMA => {
                        debug_assert!(
                            !(want_node == ExprASTWantedNode::Value
                                && cur_pt == ExprASTParseType::LambdaArguments)
                        );
                        if want_node == ExprASTWantedNode::Value {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Expected value, got comma: %.*s",
                                cur_token.start,
                            );
                            let mut cur_node =
                                new_node_with_cur_pos(K_EXPR_NODE_MISSING, cur_token, prev_token);
                            cur_node.len = 0;
                            *top_node_p = Some(cur_node);
                            want_node = ExprASTWantedNode::Operator;
                        }
                        if cur_pt == ExprASTParseType::LambdaArguments {
                            debug_assert!(!lambda_node.is_null());
                            (*lambda_node).r#type = K_EXPR_NODE_LAMBDA;
                            if let Some(colors) = &mut pstate.colors {
                                colors[(*lambda_node).data.fig().opening_hl_idx].group =
                                    hl_group!(is_invalid, c"NvimLambda", c"NvimInvalidLambda");
                            }
                        }
                        let mut valid_comma = false;
                        if ast_stack.len() >= 2 {
                            for i in 1..ast_stack.len() {
                                let eastnode_p = ast_stack[ast_stack.len() - i - 1];
                                let eastnode = (*eastnode_p).as_ref().unwrap();
                                let eastnode_type = eastnode.r#type;
                                let eastnode_lvl = node_lvl(eastnode);
                                if eastnode_type == K_EXPR_NODE_LAMBDA
                                    || eastnode_type == K_EXPR_NODE_DICT_LITERAL
                                    || eastnode_type == K_EXPR_NODE_LIST_LITERAL
                                    || eastnode_type == K_EXPR_NODE_CALL
                                {
                                    valid_comma = true;
                                    break;
                                } else if eastnode_type == K_EXPR_NODE_COMMA
                                    || eastnode_type == K_EXPR_NODE_COLON
                                    || eastnode_lvl > ExprOpLvl::Comma
                                {
                                    // Do nothing
                                } else {
                                    break;
                                }
                            }
                        }
                        if !valid_comma {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Comma outside of call, lambda or literal: %.*s",
                                cur_token.start,
                            );
                        }
                        let cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_COMMA, cur_token, prev_token);
                        is_invalid |= !viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimComma", c"NvimInvalidComma"),
                        );
                    }
                    K_EXPR_LEX_COLON => {
                        let mut is_ternary = false;
                        let mut is_subscript = false;
                        let mut valid_colon = false;
                        if ast_stack.len() >= 2 {
                            let mut can_be_ternary = true;
                            for i in 1..ast_stack.len() {
                                let eastnode_p = ast_stack[ast_stack.len() - i - 1];
                                let eastnode = (*eastnode_p).as_deref_mut().unwrap();
                                let eastnode_type = eastnode.r#type;
                                let eastnode_lvl = node_lvl(eastnode);
                                if can_be_ternary
                                    && eastnode_type == K_EXPR_NODE_TERNARY_VALUE
                                    && !eastnode.data.ter().got_colon
                                {
                                    ast_stack.truncate(ast_stack.len() - i);
                                    eastnode.start = cur_token.start;
                                    eastnode.len = cur_token.len;
                                    if prev_token.r#type == K_EXPR_LEX_SPACING {
                                        eastnode.start = prev_token.start;
                                        eastnode.len += prev_token.len;
                                    }
                                    is_ternary = true;
                                    eastnode.data.ter_mut().got_colon = true;
                                    add_value_if_missing!(c"E15: Expected value, got colon: %.*s");
                                    let child = eastnode.children.as_deref_mut().unwrap();
                                    ast_stack.push(&raw mut child.next);
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == K_EXPR_NODE_UNKNOWN_FIGURE {
                                    eastnode.r#type = K_EXPR_NODE_DICT_LITERAL;
                                    if let Some(colors) = &mut pstate.colors {
                                        colors[eastnode.data.fig().opening_hl_idx].group =
                                            hl_group!(is_invalid, c"NvimDict", c"NvimInvalidDict");
                                    }
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == K_EXPR_NODE_DICT_LITERAL {
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == K_EXPR_NODE_SUBSCRIPT {
                                    is_subscript = true;
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == K_EXPR_NODE_COLON {
                                    break;
                                } else if eastnode_lvl >= ExprOpLvl::TernaryValue {
                                    // Do nothing
                                } else if eastnode_lvl >= ExprOpLvl::Comma {
                                    can_be_ternary = false;
                                } else {
                                    break;
                                }
                            }
                        }
                        if is_subscript {
                            if want_node == ExprASTWantedNode::Value
                                && (*ast_stack[ast_stack.len() - 2]).as_ref().unwrap().r#type
                                    == K_EXPR_NODE_SUBSCRIPT
                            {
                                let mut cur_node = new_node_with_cur_pos(
                                    K_EXPR_NODE_MISSING,
                                    cur_token,
                                    prev_token,
                                );
                                cur_node.len = 0;
                                *top_node_p = Some(cur_node);
                                want_node = ExprASTWantedNode::Operator;
                            } else {
                                add_value_if_missing!(c"E15: Expected value, got colon: %.*s");
                            }
                            let cur_node =
                                new_node_with_cur_pos(K_EXPR_NODE_COLON, cur_token, prev_token);
                            is_invalid |= !viml_pexpr_handle_bop(
                                pstate,
                                &mut ast_stack,
                                cur_node,
                                &mut want_node,
                                &mut ast.err,
                            );
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(
                                    is_invalid,
                                    c"NvimSubscriptColon",
                                    c"NvimInvalidSubscriptColon"
                                ),
                            );
                        } else {
                            if !valid_colon {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Colon outside of dictionary or ternary operator: %.*s",
                                    cur_token.start,
                                );
                            }
                            add_value_if_missing!(c"E15: Expected value, got colon: %.*s");
                            if is_ternary {
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimTernaryColon",
                                        c"NvimInvalidTernaryColon"
                                    ),
                                );
                            } else {
                                let cur_node =
                                    new_node_with_cur_pos(K_EXPR_NODE_COLON, cur_token, prev_token);
                                is_invalid |= !viml_pexpr_handle_bop(
                                    pstate,
                                    &mut ast_stack,
                                    cur_node,
                                    &mut want_node,
                                    &mut ast.err,
                                );
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(is_invalid, c"NvimColon", c"NvimInvalidColon"),
                                );
                            }
                        }
                        want_node = ExprASTWantedNode::Value;
                    }
                    K_EXPR_LEX_BRACKET => {
                        if cur_token.data.brc().closing {
                            let mut new_top_node_p: *mut Option<Box<ExprASTNode>>;
                            ast_stack.pop();
                            if ast_stack.is_empty() {
                                let mut cur_node = new_node_with_cur_pos(
                                    K_EXPR_NODE_LIST_LITERAL,
                                    cur_token,
                                    prev_token,
                                );
                                cur_node.len = 0;
                                if want_node != ExprASTWantedNode::Value {
                                    cur_node.children = (*top_node_p).take();
                                }
                                *top_node_p = Some(cur_node);
                                new_top_node_p = top_node_p;
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected closing figure brace: %.*s",
                                    cur_token.start,
                                );
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(is_invalid, c"NvimList", c"NvimInvalidList"),
                                );
                            } else {
                                if want_node == ExprASTWantedNode::Value {
                                    let last_type =
                                        (**ast_stack.last().unwrap()).as_ref().unwrap().r#type;
                                    if last_type != K_EXPR_NODE_LIST_LITERAL
                                        && last_type != K_EXPR_NODE_COMMA
                                        && last_type != K_EXPR_NODE_COLON
                                    {
                                        is_invalid = true;
                                        east_set_error(
                                            pstate,
                                            &mut ast.err,
                                            c"E15: Expected value, got closing bracket: %.*s",
                                            cur_token.start,
                                        );
                                    }
                                }
                                loop {
                                    new_top_node_p = ast_stack.pop().unwrap();
                                    if ast_stack.is_empty()
                                        || (!new_top_node_p.is_null()
                                            && (*new_top_node_p).as_ref().is_some_and(|n| {
                                                n.r#type == K_EXPR_NODE_LIST_LITERAL
                                                    || n.r#type == K_EXPR_NODE_SUBSCRIPT
                                            }))
                                    {
                                        break;
                                    }
                                }
                                let new_top_node = (*new_top_node_p).as_ref().unwrap();
                                match new_top_node.r#type {
                                    K_EXPR_NODE_LIST_LITERAL => {
                                        if pt_is_assignment(cur_pt)
                                            && new_top_node.children.is_none()
                                        {
                                            is_invalid = true;
                                            east_set_error(
                                                pstate,
                                                &mut ast.err,
                                                c"E475: Unable to assign to empty list: %.*s",
                                                cur_token.start,
                                            );
                                        }
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(is_invalid, c"NvimList", c"NvimInvalidList"),
                                        );
                                    }
                                    K_EXPR_NODE_SUBSCRIPT => {
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(
                                                is_invalid,
                                                c"NvimSubscriptBracket",
                                                c"NvimInvalidSubscriptBracket"
                                            ),
                                        );
                                    }
                                    _ => {
                                        is_invalid = true;
                                        east_set_error(
                                            pstate,
                                            &mut ast.err,
                                            c"E15: Unexpected closing figure brace: %.*s",
                                            cur_token.start,
                                        );
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(is_invalid, c"NvimList", c"NvimInvalidList"),
                                        );
                                    }
                                }
                            }
                            ast_stack.push(new_top_node_p);
                            want_node = ExprASTWantedNode::Operator;
                            if ast_stack.len() <= asgn_level {
                                asgn_level = 0;
                                if cur_pt == ExprASTParseType::Expr
                                    && pt_stack.len() > 1
                                    && pt_is_assignment(pt_stack[pt_stack.len() - 2])
                                {
                                    pt_stack.pop();
                                }
                            }
                            if cur_pt == ExprASTParseType::SingleAssignment && ast_stack.len() == 1
                            {
                                pt_stack.pop();
                            }
                        } else if want_node == ExprASTWantedNode::Value {
                            let cur_node = new_node_with_cur_pos(
                                K_EXPR_NODE_LIST_LITERAL,
                                cur_token,
                                prev_token,
                            );
                            *top_node_p = Some(cur_node);
                            let node_ref = (*top_node_p).as_deref_mut().unwrap();
                            ast_stack.push(&raw mut node_ref.children);
                            if cur_pt == ExprASTParseType::Assignment {
                                pt_stack.push(ExprASTParseType::SingleAssignment);
                            } else if cur_pt == ExprASTParseType::SingleAssignment {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E475: Nested lists not allowed when assigning: %.*s",
                                    cur_token.start,
                                );
                            }
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimList", c"NvimInvalidList"),
                            );
                        } else {
                            if prev_token.r#type == K_EXPR_LEX_SPACING {
                                op_missing!();
                            }
                            let cur_node =
                                new_node_with_cur_pos(K_EXPR_NODE_SUBSCRIPT, cur_token, prev_token);
                            is_invalid |= !viml_pexpr_handle_bop(
                                pstate,
                                &mut ast_stack,
                                cur_node,
                                &mut want_node,
                                &mut ast.err,
                            );
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(
                                    is_invalid,
                                    c"NvimSubscriptBracket",
                                    c"NvimInvalidSubscriptBracket"
                                ),
                            );
                            if pt_is_assignment(cur_pt) {
                                asgn_level = ast_stack.len() - 1;
                                pt_stack.push(ExprASTParseType::Expr);
                            }
                        }
                    }
                    K_EXPR_LEX_FIGURE_BRACE => {
                        if cur_token.data.brc().closing {
                            let mut new_top_node_p: *mut Option<Box<ExprASTNode>>;
                            ast_stack.pop();
                            if ast_stack.is_empty() {
                                let mut cur_node = new_node_with_cur_pos(
                                    K_EXPR_NODE_UNKNOWN_FIGURE,
                                    cur_token,
                                    prev_token,
                                );
                                cur_node.data.fig_mut().type_guesses.allow_lambda = false;
                                cur_node.data.fig_mut().type_guesses.allow_dict = false;
                                cur_node.data.fig_mut().type_guesses.allow_ident = false;
                                cur_node.len = 0;
                                if want_node != ExprASTWantedNode::Value {
                                    cur_node.children = (*top_node_p).take();
                                }
                                *top_node_p = Some(cur_node);
                                new_top_node_p = top_node_p;
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected closing figure brace: %.*s",
                                    cur_token.start,
                                );
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimFigureBrace",
                                        c"NvimInvalidFigureBrace"
                                    ),
                                );
                            } else {
                                if want_node == ExprASTWantedNode::Value {
                                    let last_type =
                                        (**ast_stack.last().unwrap()).as_ref().unwrap().r#type;
                                    if last_type != K_EXPR_NODE_UNKNOWN_FIGURE
                                        && last_type != K_EXPR_NODE_COMMA
                                    {
                                        is_invalid = true;
                                        east_set_error(
                                            pstate,
                                            &mut ast.err,
                                            c"E15: Expected value, got closing figure brace: %.*s",
                                            cur_token.start,
                                        );
                                    }
                                }
                                loop {
                                    new_top_node_p = ast_stack.pop().unwrap();
                                    if ast_stack.is_empty()
                                        || (!new_top_node_p.is_null()
                                            && (*new_top_node_p).as_ref().is_some_and(|n| {
                                                n.r#type == K_EXPR_NODE_UNKNOWN_FIGURE
                                                    || n.r#type == K_EXPR_NODE_DICT_LITERAL
                                                    || n.r#type
                                                        == K_EXPR_NODE_CURLY_BRACES_IDENTIFIER
                                                    || n.r#type == K_EXPR_NODE_LAMBDA
                                            }))
                                    {
                                        break;
                                    }
                                }
                                let new_top_node = (*new_top_node_p).as_deref_mut().unwrap();
                                match new_top_node.r#type {
                                    K_EXPR_NODE_UNKNOWN_FIGURE => {
                                        if new_top_node.children.is_none() {
                                            new_top_node.r#type = K_EXPR_NODE_DICT_LITERAL;
                                            let grp = hl_group!(
                                                is_invalid,
                                                c"NvimDict",
                                                c"NvimInvalidDict"
                                            );
                                            if let Some(colors) = &mut pstate.colors {
                                                colors[new_top_node.data.fig().opening_hl_idx]
                                                    .group = grp;
                                            }
                                            viml_parser_highlight(
                                                pstate,
                                                cur_token.start,
                                                cur_token.len,
                                                grp,
                                            );
                                        } else if new_top_node.data.fig().type_guesses.allow_ident {
                                            new_top_node.r#type =
                                                K_EXPR_NODE_CURLY_BRACES_IDENTIFIER;
                                            let grp = hl_group!(
                                                is_invalid,
                                                c"NvimCurly",
                                                c"NvimInvalidCurly"
                                            );
                                            if let Some(colors) = &mut pstate.colors {
                                                colors[new_top_node.data.fig().opening_hl_idx]
                                                    .group = grp;
                                            }
                                            viml_parser_highlight(
                                                pstate,
                                                cur_token.start,
                                                cur_token.len,
                                                grp,
                                            );
                                        } else {
                                            is_invalid = true;
                                            east_set_error(
                                                pstate,
                                                &mut ast.err,
                                                c"E15: Don't know what figure brace means: %.*s",
                                                new_top_node.start,
                                            );
                                            let grp = hl_group!(
                                                is_invalid,
                                                c"NvimFigureBrace",
                                                c"NvimInvalidFigureBrace"
                                            );
                                            if let Some(colors) = &mut pstate.colors {
                                                colors[new_top_node.data.fig().opening_hl_idx]
                                                    .group = grp;
                                            }
                                            viml_parser_highlight(
                                                pstate,
                                                cur_token.start,
                                                cur_token.len,
                                                grp,
                                            );
                                        }
                                    }
                                    K_EXPR_NODE_DICT_LITERAL => {
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(is_invalid, c"NvimDict", c"NvimInvalidDict"),
                                        );
                                    }
                                    K_EXPR_NODE_CURLY_BRACES_IDENTIFIER => {
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(
                                                is_invalid,
                                                c"NvimCurly",
                                                c"NvimInvalidCurly"
                                            ),
                                        );
                                    }
                                    K_EXPR_NODE_LAMBDA => {
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(
                                                is_invalid,
                                                c"NvimLambda",
                                                c"NvimInvalidLambda"
                                            ),
                                        );
                                    }
                                    _ => {
                                        is_invalid = true;
                                        east_set_error(
                                            pstate,
                                            &mut ast.err,
                                            c"E15: Unexpected closing figure brace: %.*s",
                                            cur_token.start,
                                        );
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(
                                                is_invalid,
                                                c"NvimFigureBrace",
                                                c"NvimInvalidFigureBrace"
                                            ),
                                        );
                                    }
                                }
                            }
                            ast_stack.push(new_top_node_p);
                            want_node = ExprASTWantedNode::Operator;
                            if ast_stack.len() <= asgn_level
                                && cur_pt == ExprASTParseType::Expr
                                && pt_stack.len() > 1
                                && pt_is_assignment(pt_stack[pt_stack.len() - 2])
                            {
                                pt_stack.pop();
                                asgn_level = 0;
                            }
                        } else {
                            if want_node == ExprASTWantedNode::Value {
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimFigureBrace",
                                        c"NvimInvalidFigureBrace"
                                    ),
                                );
                                let mut cur_node = if pt_is_assignment(cur_pt) {
                                    let mut n = new_node_with_cur_pos(
                                        K_EXPR_NODE_CURLY_BRACES_IDENTIFIER,
                                        cur_token,
                                        prev_token,
                                    );
                                    n.data.fig_mut().type_guesses.allow_lambda = false;
                                    n.data.fig_mut().type_guesses.allow_dict = false;
                                    n.data.fig_mut().type_guesses.allow_ident = true;
                                    pt_stack.push(ExprASTParseType::Expr);
                                    n
                                } else {
                                    let mut n = new_node_with_cur_pos(
                                        K_EXPR_NODE_UNKNOWN_FIGURE,
                                        cur_token,
                                        prev_token,
                                    );
                                    n.data.fig_mut().type_guesses.allow_lambda = true;
                                    n.data.fig_mut().type_guesses.allow_dict = true;
                                    n.data.fig_mut().type_guesses.allow_ident = true;
                                    n
                                };
                                if let Some(colors) = &pstate.colors {
                                    cur_node.data.fig_mut().opening_hl_idx = colors.len() - 1;
                                }
                                *top_node_p = Some(cur_node);
                                let node_ref = (*top_node_p).as_deref_mut().unwrap();
                                ast_stack.push(&raw mut node_ref.children);
                                pt_stack.push(ExprASTParseType::LambdaArguments);
                                lambda_node = &raw mut *node_ref;
                            } else {
                                if prev_token.r#type == K_EXPR_LEX_SPACING {
                                    op_missing!();
                                }
                                let top_type = (*top_node_p).as_ref().unwrap().r#type;
                                if top_type == K_EXPR_NODE_COMPLEX_IDENTIFIER
                                    || top_type == K_EXPR_NODE_PLAIN_IDENTIFIER
                                    || top_type == K_EXPR_NODE_CURLY_BRACES_IDENTIFIER
                                {
                                    let mut comp_node = new_node_with_cur_pos(
                                        K_EXPR_NODE_COMPLEX_IDENTIFIER,
                                        cur_token,
                                        prev_token,
                                    );
                                    comp_node.len = 0;
                                    comp_node.children = (*top_node_p).take();
                                    *top_node_p = Some(comp_node);
                                    let comp_ref = (*top_node_p).as_deref_mut().unwrap();
                                    let comp_child = comp_ref.children.as_deref_mut().unwrap();
                                    ast_stack.push(&raw mut comp_child.next);
                                    let new_top_node_p = *ast_stack.last().unwrap();
                                    let mut cur_node = new_node_with_cur_pos(
                                        K_EXPR_NODE_CURLY_BRACES_IDENTIFIER,
                                        cur_token,
                                        prev_token,
                                    );
                                    if let Some(colors) = &pstate.colors {
                                        cur_node.data.fig_mut().opening_hl_idx = colors.len();
                                    }
                                    cur_node.data.fig_mut().type_guesses.allow_lambda = false;
                                    cur_node.data.fig_mut().type_guesses.allow_dict = false;
                                    cur_node.data.fig_mut().type_guesses.allow_ident = true;
                                    if pt_is_assignment(cur_pt) {
                                        pt_stack.push(ExprASTParseType::Expr);
                                    }
                                    want_node = ExprASTWantedNode::Value;
                                    *new_top_node_p = Some(cur_node);
                                    let cur_ref = (*new_top_node_p).as_deref_mut().unwrap();
                                    ast_stack.push(&raw mut cur_ref.children);
                                    viml_parser_highlight(
                                        pstate,
                                        cur_token.start,
                                        cur_token.len,
                                        hl_group!(is_invalid, c"NvimCurly", c"NvimInvalidCurly"),
                                    );
                                } else {
                                    op_missing!();
                                }
                            }
                            if pt_is_assignment(cur_pt)
                                && !pt_is_assignment(*pt_stack.last().unwrap())
                            {
                                asgn_level = ast_stack.len() - 1;
                            }
                        }
                    }
                    K_EXPR_LEX_ARROW => {
                        if cur_pt == ExprASTParseType::LambdaArguments {
                            pt_stack.pop();
                            if want_node == ExprASTWantedNode::Value {
                                ast_stack.pop();
                            }
                            while (**ast_stack.last().unwrap()).as_ref().unwrap().r#type
                                != K_EXPR_NODE_LAMBDA
                                && (**ast_stack.last().unwrap()).as_ref().unwrap().r#type
                                    != K_EXPR_NODE_UNKNOWN_FIGURE
                            {
                                ast_stack.pop();
                            }
                            debug_assert!(std::ptr::eq(
                                (**ast_stack.last().unwrap()).as_deref().unwrap(),
                                lambda_node
                            ));
                            (*lambda_node).r#type = K_EXPR_NODE_LAMBDA;
                            if let Some(colors) = &mut pstate.colors {
                                colors[(*lambda_node).data.fig().opening_hl_idx].group =
                                    hl_group!(is_invalid, c"NvimLambda", c"NvimInvalidLambda");
                            }
                            let cur_node =
                                new_node_with_cur_pos(K_EXPR_NODE_ARROW, cur_token, prev_token);
                            if (*lambda_node).children.is_none() {
                                (*lambda_node).children = Some(cur_node);
                                let children_slot: *mut Option<Box<ExprASTNode>> =
                                    &raw mut (*lambda_node).children;
                                let arrow_ref = (*children_slot).as_deref_mut().unwrap();
                                ast_stack.push(children_slot);
                                ast_stack.push(&raw mut arrow_ref.children);
                            } else {
                                let lchild = (*lambda_node).children.as_deref_mut().unwrap();
                                lchild.next = Some(cur_node);
                                let next_slot: *mut Option<Box<ExprASTNode>> = &raw mut lchild.next;
                                let arrow_ref = (*next_slot).as_deref_mut().unwrap();
                                ast_stack.push(next_slot);
                                ast_stack.push(&raw mut arrow_ref.children);
                            }
                            lambda_node = std::ptr::null_mut();
                        } else {
                            is_invalid = true;
                            if want_node == ExprASTWantedNode::Value {
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected arrow: %.*s",
                                    cur_token.start,
                                );
                                let mut missing_node = new_node_with_cur_pos(
                                    K_EXPR_NODE_MISSING,
                                    cur_token,
                                    prev_token,
                                );
                                missing_node.len = 0;
                                *top_node_p = Some(missing_node);
                                want_node = ExprASTWantedNode::Operator;
                            }
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Arrow outside of lambda: %.*s",
                                cur_token.start,
                            );
                            let cur_node =
                                new_node_with_cur_pos(K_EXPR_NODE_ARROW, cur_token, prev_token);
                            let _ = viml_pexpr_handle_bop(
                                pstate,
                                &mut ast_stack,
                                cur_node,
                                &mut want_node,
                                &mut ast.err,
                            );
                        }
                        want_node = ExprASTWantedNode::Value;
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimArrow", c"NvimInvalidArrow"),
                        );
                    }
                    K_EXPR_LEX_PLAIN_IDENTIFIER => {
                        let scope = if cur_token.r#type == K_EXPR_LEX_INVALID {
                            K_EXPR_VAR_SCOPE_MISSING
                        } else {
                            cur_token.data.var().scope
                        };
                        if want_node == ExprASTWantedNode::Value {
                            want_node = ExprASTWantedNode::Operator;
                            let mut cur_node = new_node_with_cur_pos(
                                if node_is_key {
                                    K_EXPR_NODE_PLAIN_KEY
                                } else {
                                    K_EXPR_NODE_PLAIN_IDENTIFIER
                                },
                                cur_token,
                                prev_token,
                            );
                            cur_node.data.var_mut().scope = scope;
                            let scope_shift = if scope == K_EXPR_VAR_SCOPE_MISSING {
                                0
                            } else {
                                2
                            };
                            cur_node.data.var_mut().ident =
                                pline.data.add(cur_token.start.col + scope_shift);
                            cur_node.data.var_mut().ident_len = cur_token.len - scope_shift;
                            *top_node_p = Some(cur_node);
                            if scope_shift != 0 {
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    1,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimIdentifierScope",
                                        c"NvimInvalidIdentifierScope"
                                    ),
                                );
                                viml_parser_highlight(
                                    pstate,
                                    shifted_pos(cur_token.start, 1),
                                    1,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimIdentifierScopeDelimiter",
                                        c"NvimInvalidIdentifierScopeDelimiter"
                                    ),
                                );
                            }
                            viml_parser_highlight(
                                pstate,
                                shifted_pos(cur_token.start, scope_shift),
                                cur_token.len - scope_shift,
                                if node_is_key {
                                    hl_group!(
                                        is_invalid,
                                        c"NvimIdentifierKey",
                                        c"NvimInvalidIdentifierKey"
                                    )
                                } else {
                                    hl_group!(
                                        is_invalid,
                                        c"NvimIdentifierName",
                                        c"NvimInvalidIdentifierName"
                                    )
                                },
                            );
                        } else if scope == K_EXPR_VAR_SCOPE_MISSING {
                            if prev_token.r#type == K_EXPR_LEX_SPACING {
                                op_missing!();
                            }
                            let top_type = (*top_node_p).as_ref().unwrap().r#type;
                            if top_type == K_EXPR_NODE_COMPLEX_IDENTIFIER
                                || top_type == K_EXPR_NODE_PLAIN_IDENTIFIER
                                || top_type == K_EXPR_NODE_CURLY_BRACES_IDENTIFIER
                            {
                                let mut comp_node = new_node_with_cur_pos(
                                    K_EXPR_NODE_COMPLEX_IDENTIFIER,
                                    cur_token,
                                    prev_token,
                                );
                                comp_node.len = 0;
                                comp_node.children = (*top_node_p).take();
                                *top_node_p = Some(comp_node);
                                let comp_ref = (*top_node_p).as_deref_mut().unwrap();
                                let comp_child = comp_ref.children.as_deref_mut().unwrap();
                                ast_stack.push(&raw mut comp_child.next);
                                let new_top_node_p = *ast_stack.last().unwrap();
                                let mut cur_node = new_node_with_cur_pos(
                                    K_EXPR_NODE_PLAIN_IDENTIFIER,
                                    cur_token,
                                    prev_token,
                                );
                                cur_node.data.var_mut().scope = scope;
                                cur_node.data.var_mut().ident = pline.data.add(cur_token.start.col);
                                cur_node.data.var_mut().ident_len = cur_token.len;
                                want_node = ExprASTWantedNode::Operator;
                                *new_top_node_p = Some(cur_node);
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimIdentifierName",
                                        c"NvimInvalidIdentifierName"
                                    ),
                                );
                            } else {
                                op_missing!();
                            }
                        } else {
                            op_missing!();
                        }
                    }
                    K_EXPR_LEX_NUMBER => {
                        if want_node != ExprASTWantedNode::Value {
                            op_missing!();
                        }
                        let cur_node = if node_is_key {
                            let mut n =
                                new_node_with_cur_pos(K_EXPR_NODE_PLAIN_KEY, cur_token, prev_token);
                            n.data.var_mut().ident = pline.data.add(cur_token.start.col);
                            n.data.var_mut().ident_len = cur_token.len;
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(
                                    is_invalid,
                                    c"NvimIdentifierKey",
                                    c"NvimInvalidIdentifierKey"
                                ),
                            );
                            n
                        } else if cur_token.data.num().is_float {
                            let mut n =
                                new_node_with_cur_pos(K_EXPR_NODE_FLOAT, cur_token, prev_token);
                            n.data.flt_mut().value = cur_token.data.num().val.floating();
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimFloat", c"NvimInvalidFloat"),
                            );
                            n
                        } else {
                            let mut n =
                                new_node_with_cur_pos(K_EXPR_NODE_INTEGER, cur_token, prev_token);
                            n.data.num_mut().value = cur_token.data.num().val.integer();
                            let prefix_length =
                                BASE_TO_PREFIX_LENGTH[cur_token.data.num().base as usize];
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                prefix_length,
                                hl_group!(
                                    is_invalid,
                                    c"NvimNumberPrefix",
                                    c"NvimInvalidNumberPrefix"
                                ),
                            );
                            viml_parser_highlight(
                                pstate,
                                shifted_pos(cur_token.start, prefix_length),
                                cur_token.len - prefix_length,
                                hl_group!(is_invalid, c"NvimNumber", c"NvimInvalidNumber"),
                            );
                            n
                        };
                        want_node = ExprASTWantedNode::Operator;
                        *top_node_p = Some(cur_node);
                    }
                    K_EXPR_LEX_DOT => {
                        add_value_if_missing!(c"E15: Unexpected dot: %.*s");
                        let cur_node = if prev_token.r#type == K_EXPR_LEX_SPACING {
                            if cur_pt == ExprASTParseType::Assignment {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Cannot concatenate in assignments: %.*s",
                                    cur_token.start,
                                );
                            }
                            let n =
                                new_node_with_cur_pos(K_EXPR_NODE_CONCAT, cur_token, prev_token);
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimConcat", c"NvimInvalidConcat"),
                            );
                            n
                        } else {
                            let n = new_node_with_cur_pos(
                                K_EXPR_NODE_CONCAT_OR_SUBSCRIPT,
                                cur_token,
                                prev_token,
                            );
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(
                                    is_invalid,
                                    c"NvimConcatOrSubscript",
                                    c"NvimInvalidConcatOrSubscript"
                                ),
                            );
                            n
                        };
                        let _ = viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                    }
                    K_EXPR_LEX_PARENTHESIS => {
                        if cur_token.data.brc().closing {
                            let mut no_paren_closing_error = false;
                            if want_node == ExprASTWantedNode::Value {
                                if ast_stack.len() > 1 {
                                    let prev_top_node =
                                        (*ast_stack[ast_stack.len() - 2]).as_ref().unwrap();
                                    if prev_top_node.r#type == K_EXPR_NODE_CALL {
                                        ast_stack.pop();
                                        no_paren_closing_error = true;
                                    }
                                }
                                if !no_paren_closing_error {
                                    is_invalid = true;
                                    east_set_error(
                                        pstate,
                                        &mut ast.err,
                                        c"E15: Expected value, got parenthesis: %.*s",
                                        cur_token.start,
                                    );
                                    let mut cur_node = new_node_with_cur_pos(
                                        K_EXPR_NODE_MISSING,
                                        cur_token,
                                        prev_token,
                                    );
                                    cur_node.len = 0;
                                    *top_node_p = Some(cur_node);
                                }
                            } else {
                                ast_stack.pop();
                            }
                            let mut new_top_node_p: *mut Option<Box<ExprASTNode>> =
                                std::ptr::null_mut();
                            while !ast_stack.is_empty()
                                && (new_top_node_p.is_null()
                                    || (*new_top_node_p).as_ref().is_none_or(|n| {
                                        n.r#type != K_EXPR_NODE_NESTED
                                            && n.r#type != K_EXPR_NODE_CALL
                                    }))
                            {
                                new_top_node_p = ast_stack.pop().unwrap();
                            }
                            if !new_top_node_p.is_null()
                                && (*new_top_node_p).as_ref().is_some_and(|n| {
                                    n.r#type == K_EXPR_NODE_NESTED || n.r#type == K_EXPR_NODE_CALL
                                })
                            {
                                if (*new_top_node_p).as_ref().unwrap().r#type == K_EXPR_NODE_NESTED
                                {
                                    viml_parser_highlight(
                                        pstate,
                                        cur_token.start,
                                        cur_token.len,
                                        hl_group!(
                                            is_invalid,
                                            c"NvimNestingParenthesis",
                                            c"NvimInvalidNestingParenthesis"
                                        ),
                                    );
                                } else {
                                    viml_parser_highlight(
                                        pstate,
                                        cur_token.start,
                                        cur_token.len,
                                        hl_group!(
                                            is_invalid,
                                            c"NvimCallingParenthesis",
                                            c"NvimInvalidCallingParenthesis"
                                        ),
                                    );
                                }
                            } else {
                                if new_top_node_p.is_null() {
                                    new_top_node_p = top_node_p;
                                }
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected closing parenthesis: %.*s",
                                    cur_token.start,
                                );
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(
                                        is_invalid,
                                        c"NvimNestingParenthesis",
                                        c"NvimInvalidNestingParenthesis"
                                    ),
                                );
                                let mut cur_node = viml_pexpr_new_node(K_EXPR_NODE_NESTED);
                                cur_node.start = cur_token.start;
                                cur_node.len = 0;
                                cur_node.children = (*new_top_node_p).take();
                                *new_top_node_p = Some(cur_node);
                            }
                            ast_stack.push(new_top_node_p);
                            want_node = ExprASTWantedNode::Operator;
                        } else {
                            match want_node {
                                ExprASTWantedNode::Value => {
                                    let cur_node = new_node_with_cur_pos(
                                        K_EXPR_NODE_NESTED,
                                        cur_token,
                                        prev_token,
                                    );
                                    *top_node_p = Some(cur_node);
                                    let node_ref = (*top_node_p).as_deref_mut().unwrap();
                                    ast_stack.push(&raw mut node_ref.children);
                                    viml_parser_highlight(
                                        pstate,
                                        cur_token.start,
                                        cur_token.len,
                                        hl_group!(
                                            is_invalid,
                                            c"NvimNestingParenthesis",
                                            c"NvimInvalidNestingParenthesis"
                                        ),
                                    );
                                }
                                ExprASTWantedNode::Operator => {
                                    if prev_token.r#type == K_EXPR_LEX_SPACING {
                                        let top_type = (*top_node_p).as_ref().unwrap().r#type;
                                        if top_type != K_EXPR_NODE_PLAIN_IDENTIFIER
                                            && top_type != K_EXPR_NODE_COMPLEX_IDENTIFIER
                                            && top_type != K_EXPR_NODE_CURLY_BRACES_IDENTIFIER
                                        {
                                            op_missing!();
                                        }
                                    }
                                    let cur_node = new_node_with_cur_pos(
                                        K_EXPR_NODE_CALL,
                                        cur_token,
                                        prev_token,
                                    );
                                    is_invalid |= !viml_pexpr_handle_bop(
                                        pstate,
                                        &mut ast_stack,
                                        cur_node,
                                        &mut want_node,
                                        &mut ast.err,
                                    );
                                    viml_parser_highlight(
                                        pstate,
                                        cur_token.start,
                                        cur_token.len,
                                        hl_group!(
                                            is_invalid,
                                            c"NvimCallingParenthesis",
                                            c"NvimInvalidCallingParenthesis"
                                        ),
                                    );
                                }
                            }
                            want_node = ExprASTWantedNode::Value;
                        }
                    }
                    K_EXPR_LEX_QUESTION => {
                        add_value_if_missing!(c"E15: Expected value, got question mark: %.*s");
                        let cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_TERNARY, cur_token, prev_token);
                        is_invalid |= !viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimTernary", c"NvimInvalidTernary"),
                        );
                        let mut ter_val_node =
                            new_node_with_cur_pos(K_EXPR_NODE_TERNARY_VALUE, cur_token, prev_token);
                        ter_val_node.data.ter_mut().got_colon = false;
                        let slot = *ast_stack.last().unwrap();
                        *slot = Some(ter_val_node);
                        let ter_ref = (*slot).as_deref_mut().unwrap();
                        ast_stack.push(&raw mut ter_ref.children);
                    }
                    K_EXPR_LEX_DOUBLE_QUOTED_STRING | K_EXPR_LEX_SINGLE_QUOTED_STRING => {
                        let is_double = tok_type == K_EXPR_LEX_DOUBLE_QUOTED_STRING;
                        if !cur_token.data.str().closed {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                if is_double {
                                    c"E114: Missing double quote: %.*s"
                                } else {
                                    c"E115: Missing single quote: %.*s"
                                },
                                cur_token.start,
                            );
                        }
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let mut cur_node = new_node_with_cur_pos(
                            if is_double {
                                K_EXPR_NODE_DOUBLE_QUOTED_STRING
                            } else {
                                K_EXPR_NODE_SINGLE_QUOTED_STRING
                            },
                            cur_token,
                            prev_token,
                        );
                        parse_quoted_string(
                            pstate,
                            &mut cur_node,
                            cur_token,
                            &ast_stack,
                            is_invalid,
                        );
                        *top_node_p = Some(cur_node);
                        want_node = ExprASTWantedNode::Operator;
                    }
                    K_EXPR_LEX_ASSIGNMENT => {
                        if cur_pt == ExprASTParseType::Assignment {
                            pt_stack.pop();
                        } else if cur_pt == ExprASTParseType::SingleAssignment {
                            pt_stack.truncate(pt_stack.len() - 2);
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E475: Expected closing bracket to end list assignment lvalue: %.*s",
                                cur_token.start,
                            );
                        } else {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Misplaced assignment: %.*s",
                                cur_token.start,
                            );
                        }
                        add_value_if_missing!(c"E15: Unexpected assignment: %.*s");
                        let mut cur_node =
                            new_node_with_cur_pos(K_EXPR_NODE_ASSIGNMENT, cur_token, prev_token);
                        cur_node.data.ass_mut().r#type = cur_token.data.ass().r#type;
                        let hl_grp = match cur_token.data.ass().r#type {
                            K_EXPR_ASGN_PLAIN => hl_group!(
                                is_invalid,
                                c"NvimPlainAssignment",
                                c"NvimInvalidPlainAssignment"
                            ),
                            K_EXPR_ASGN_ADD => hl_group!(
                                is_invalid,
                                c"NvimAssignmentWithAddition",
                                c"NvimInvalidAssignmentWithAddition"
                            ),
                            K_EXPR_ASGN_SUBTRACT => hl_group!(
                                is_invalid,
                                c"NvimAssignmentWithSubtraction",
                                c"NvimInvalidAssignmentWithSubtraction"
                            ),
                            _ => hl_group!(
                                is_invalid,
                                c"NvimAssignmentWithConcatenation",
                                c"NvimInvalidAssignmentWithConcatenation"
                            ),
                        };
                        viml_parser_highlight(pstate, cur_token.start, cur_token.len, hl_grp);
                        let _ = viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                    }
                    _ => {}
                }
                break 'process_token;
            }
            prev_token = cur_token;
            highlighted_prev_spacing = false;
            viml_parser_advance(pstate, cur_token.len);
        }

        if want_node == ExprASTWantedNode::Value
            && *pt_stack.last().unwrap() != ExprASTParseType::LambdaArguments
        {
            east_set_error(
                pstate,
                &mut ast.err,
                c"E15: Expected value, got EOC: %.*s",
                pstate.pos,
            );
        } else if ast_stack.len() != 1 {
            ast_stack.pop();
            while ast.err.msg.is_none()
                && let Some(node_p) = ast_stack.pop()
            {
                let cur_node = (*node_p).as_deref_mut().unwrap();
                match cur_node.r#type {
                    K_EXPR_NODE_OP_MISSING | K_EXPR_NODE_MISSING => {}
                    K_EXPR_NODE_CALL => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E116: Missing closing parenthesis for function call: %.*s",
                            cur_node.start,
                        );
                    }
                    K_EXPR_NODE_NESTED => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E110: Missing closing parenthesis for nested expression: %.*s",
                            cur_node.start,
                        );
                    }
                    K_EXPR_NODE_LIST_LITERAL => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E697: Missing end of List ']': %.*s",
                            cur_node.start,
                        );
                    }
                    K_EXPR_NODE_DICT_LITERAL => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E723: Missing end of Dictionary '}': %.*s",
                            cur_node.start,
                        );
                    }
                    K_EXPR_NODE_UNKNOWN_FIGURE => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E15: Missing closing figure brace: %.*s",
                            cur_node.start,
                        );
                    }
                    K_EXPR_NODE_LAMBDA => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E15: Missing closing figure brace for lambda: %.*s",
                            cur_node.start,
                        );
                    }
                    K_EXPR_NODE_CURLY_BRACES_IDENTIFIER
                    | K_EXPR_NODE_INTEGER
                    | K_EXPR_NODE_FLOAT
                    | K_EXPR_NODE_SINGLE_QUOTED_STRING
                    | K_EXPR_NODE_DOUBLE_QUOTED_STRING
                    | K_EXPR_NODE_OPTION
                    | K_EXPR_NODE_ENVIRONMENT
                    | K_EXPR_NODE_REGISTER
                    | K_EXPR_NODE_PLAIN_IDENTIFIER
                    | K_EXPR_NODE_PLAIN_KEY => {
                        std::process::abort();
                    }
                    K_EXPR_NODE_TERNARY_VALUE if !cur_node.data.ter_mut().got_colon => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E109: Missing ':' after '?': %.*s",
                            cur_node.start,
                        );
                    }
                    _ => {}
                }
            }
        }
        ast
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viml::parser::parser::{ParserHighlight, ParserInputReader};

    fn with_pstate<R>(input: &[u8], f: impl FnOnce(&mut ParserState<'_>) -> R) -> R {
        let lines = [
            ParserLine {
                data: input.as_ptr().cast::<c_char>(),
                size: input.len(),
            },
            ParserLine {
                data: std::ptr::null(),
                size: 0,
            },
        ];
        let mut pstate = ParserState {
            reader: ParserInputReader::new(&lines),
            pos: ParserPosition { line: 0, col: 0 },
            colors: Some(ParserHighlight::new()),
        };
        f(&mut pstate)
    }

    fn lex_one(input: &[u8], flags: c_int) -> LexExprToken {
        with_pstate(input, |pstate| viml_pexpr_next_token(pstate, flags))
    }

    #[test]
    fn lexer_tokens_and_flags() {
        let t = lex_one(b"(", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_PARENTHESIS);
        assert!(!t.data.brc().closing);

        let t = lex_one(b")]", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_PARENTHESIS);
        assert!(t.data.brc().closing);
        assert_eq!(t.len, 1);

        let t = lex_one(b"0x123", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_NUMBER);
        assert!(!t.data.num().is_float);
        assert_eq!(t.data.num().base, 16);
        assert_eq!(t.data.num().val.integer(), 0x123);

        let t = lex_one(b"0b100111", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_NUMBER);
        assert_eq!(t.data.num().base, 2);
        assert_eq!(t.data.num().val.integer(), 39);

        let t = lex_one(b"0123", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_NUMBER);
        assert_eq!(t.data.num().base, 8);
        assert_eq!(t.data.num().val.integer(), 83);

        // Without K_ELFLAG_ALLOW_FLOAT, "2.5" stops at "2".
        let t = lex_one(b"2.5", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_NUMBER);
        assert!(!t.data.num().is_float);
        assert_eq!(t.len, 1);
        assert_eq!(t.data.num().val.integer(), 2);

        // With K_ELFLAG_ALLOW_FLOAT, "2.5e-2" parses as float.
        let t = lex_one(b"2.5e-2", K_ELFLAG_ALLOW_FLOAT);
        assert_eq!(t.r#type, K_EXPR_LEX_NUMBER);
        assert!(t.data.num().is_float);
        assert_eq!(t.len, 6);
        assert!((t.data.num().val.floating() - 0.025).abs() < 1e-12);

        // Comparisons and case strategies
        let t = lex_one(b"==#", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_COMPARISON);
        assert_eq!(t.data.cmp().r#type, K_EXPR_CMP_EQUAL);
        assert!(!t.data.cmp().inv);
        assert_eq!(t.data.cmp().ccs, K_CCSTRATEGY_MATCH_CASE);

        let t = lex_one(b"!~?", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_COMPARISON);
        assert_eq!(t.data.cmp().r#type, K_EXPR_CMP_MATCHES);
        assert!(t.data.cmp().inv);
        assert_eq!(t.data.cmp().ccs, K_CCSTRATEGY_IGNORE_CASE);

        // `is` / `isnot` with and without K_ELFLAG_IS_NOT_CMP
        let t = lex_one(b"isnot#", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_COMPARISON);
        assert_eq!(t.data.cmp().r#type, K_EXPR_CMP_IDENTICAL);
        assert!(t.data.cmp().inv);
        assert_eq!(t.data.cmp().ccs, K_CCSTRATEGY_MATCH_CASE);

        let t = lex_one(b"isnot#foo", K_ELFLAG_IS_NOT_CMP);
        assert_eq!(t.r#type, K_EXPR_LEX_PLAIN_IDENTIFIER);
        assert_eq!(t.len, 9);
        assert!(t.data.var().autoload);

        // Scope and K_ELFLAG_FORBID_SCOPE
        let t = lex_one(b"g:test#var", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_PLAIN_IDENTIFIER);
        assert_eq!(t.data.var().scope, b'g' as c_int);
        assert!(t.data.var().autoload);

        let t = lex_one(b"g:test", K_ELFLAG_FORBID_SCOPE);
        assert_eq!(t.r#type, K_EXPR_LEX_PLAIN_IDENTIFIER);
        assert_eq!(t.len, 1);
        assert_eq!(t.data.var().scope, 0);

        // Options, registers, env, strings, assignments
        let t = lex_one(b"&l:opt", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_OPTION);
        assert_eq!(t.data.opt().scope, b'l' as c_int);
        assert_eq!(t.data.opt().len, 3);

        let t = lex_one(b"&g:", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_INVALID);
        assert_eq!(
            &t.data.err().msg.unwrap().to_bytes()[4..],
            b": Option name missing: %.*s"
        );

        let t = lex_one(b"@a", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_REGISTER);
        assert_eq!(t.data.reg().name, b'a' as c_int);

        let t = lex_one(b"'x'''", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_SINGLE_QUOTED_STRING);
        assert!(t.data.str().closed);

        let t = lex_one(b"\"x\\\"", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_DOUBLE_QUOTED_STRING);
        assert!(!t.data.str().closed);

        let t = lex_one(b".=", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_ASSIGNMENT);
        assert_eq!(t.data.ass().r#type, K_EXPR_ASGN_CONCAT);

        // Peek does not advance pstate.pos
        with_pstate(b"abc", |pstate| {
            let t = viml_pexpr_next_token(pstate, K_ELFLAG_PEEK);
            assert_eq!(t.r#type, K_EXPR_LEX_PLAIN_IDENTIFIER);
            assert_eq!(t.len, 3);
            assert_eq!(pstate.pos.col, 0);
        });

        // EOC and K_ELFLAG_FORBID_EOC
        let t = lex_one(b"|", 0);
        assert_eq!(t.r#type, K_EXPR_LEX_EOC);
        let t = lex_one(b"|", K_ELFLAG_FORBID_EOC);
        assert_eq!(t.r#type, K_EXPR_LEX_INVALID);
    }

    #[test]
    fn lex_and_parse_expressions_and_errors() {
        // Binary precedence: 1 + 2 * 3
        with_pstate(b"1 + 2 * 3", |pstate| {
            let ast = viml_pexpr_parse(pstate, 0);
            assert!(ast.err.msg.is_none());
            let root = ast.root.as_ref().unwrap();
            assert_eq!(root.r#type, K_EXPR_NODE_BINARY_PLUS);
            let left = root.children.as_ref().unwrap();
            let right = left.next.as_ref().unwrap();
            assert_eq!(left.r#type, K_EXPR_NODE_INTEGER);
            assert_eq!(right.r#type, K_EXPR_NODE_MULTIPLICATION);
            assert!(!pstate.colors.as_ref().unwrap().is_empty());
        });

        // Ternary, lambda, dictionary, list, call, subscript
        with_pstate(b"cond ? {x -> [x, {'k': f(x)[0]}]} : 0", |pstate| {
            let ast = viml_pexpr_parse(pstate, 0);
            assert!(ast.err.msg.is_none());
            assert_eq!(ast.root.as_ref().unwrap().r#type, K_EXPR_NODE_TERNARY);
        });

        // Assignment with K_EXPR_FLAGS_PARSE_LET
        with_pstate(b"[a, b] = [1, 2]", |pstate| {
            let ast = viml_pexpr_parse(pstate, K_EXPR_FLAGS_PARSE_LET);
            assert!(ast.err.msg.is_none());
            assert_eq!(ast.root.as_ref().unwrap().r#type, K_EXPR_NODE_ASSIGNMENT);
            assert_eq!(
                ast.root.as_ref().unwrap().data.ass().r#type,
                K_EXPR_ASGN_PLAIN
            );
        });

        with_pstate(b"a += 1", |pstate| {
            let ast = viml_pexpr_parse(pstate, K_EXPR_FLAGS_PARSE_LET);
            assert!(ast.err.msg.is_none());
            assert_eq!(ast.root.as_ref().unwrap().r#type, K_EXPR_NODE_ASSIGNMENT);
            assert_eq!(
                ast.root.as_ref().unwrap().data.ass().r#type,
                K_EXPR_ASGN_ADD
            );
        });

        // Syntax errors report expected diagnostics
        with_pstate(b"1 + (2 * 3", |pstate| {
            let ast = viml_pexpr_parse(pstate, 0);
            assert_eq!(
                &ast.err.msg.unwrap().to_bytes()[4..],
                b": Missing closing parenthesis for nested expression: %.*s"
            );
        });

        with_pstate(b"cond ? 1", |pstate| {
            let ast = viml_pexpr_parse(pstate, 0);
            assert_eq!(
                &ast.err.msg.unwrap().to_bytes()[4..],
                b": Missing ':' after '?': %.*s"
            );
        });
    }
}
