//! Vimscript expression lexer and parser (`src/nvim/viml/parser/expressions.c`).

use crate::charset::{
    ascii_isalpha, ascii_isdigit, ascii_iswhite, ascii_isxdigit, uvarnumber_T, vim_str2nr,
};
use crate::keycodes::{FSK_IN_STRING, FSK_KEYCODE, FSK_SIMPLIFY, NUL, trans_special};
use crate::mbyte::{mb_copy_char, utf_char2bytes, utf_char2len, utfc_ptr2len_len};
use crate::memory::{xfree, xmalloc, xmallocz};
use crate::viml::parser::parser::{
    ParserHighlightChunk, ParserLine, ParserPosition, ParserState, viml_parser_get_remaining_line,
};
use std::ffi::{c_char, c_int};

use crate::eval::STR2NR_ALL;
use crate::eval::typval::float_T;
use crate::testing::FF;

pub(crate) const BS: u8 = 0x08;
pub(crate) const TAB: u8 = 0x09;
pub(crate) const NL: u8 = 0x0a;
pub(crate) const CAR: u8 = 0x0d;
pub(crate) const ESC: u8 = 0x1b;
pub(crate) const AUTOLOAD_CHAR: u8 = b'#';

const Ctrl_A: u8 = 1;
pub(crate) const Ctrl_H: u8 = 8;
const Ctrl_K: u8 = 11;
const Ctrl_Z: u8 = 26;

pub type ExprCaseCompareStrategy = c_int;
pub const kCCStrategyUseOption: ExprCaseCompareStrategy = 0;
pub const kCCStrategyMatchCase: ExprCaseCompareStrategy = b'#' as c_int;
pub const kCCStrategyIgnoreCase: ExprCaseCompareStrategy = b'?' as c_int;

pub type LexExprTokenType = c_int;
pub const kExprLexInvalid: LexExprTokenType = 0;
pub const kExprLexMissing: LexExprTokenType = 1;
pub const kExprLexSpacing: LexExprTokenType = 2;
pub const kExprLexEOC: LexExprTokenType = 3;
pub const kExprLexQuestion: LexExprTokenType = 4;
pub const kExprLexColon: LexExprTokenType = 5;
pub const kExprLexOr: LexExprTokenType = 6;
pub const kExprLexAnd: LexExprTokenType = 7;
pub const kExprLexComparison: LexExprTokenType = 8;
pub const kExprLexPlus: LexExprTokenType = 9;
pub const kExprLexMinus: LexExprTokenType = 10;
pub const kExprLexDot: LexExprTokenType = 11;
pub const kExprLexMultiplication: LexExprTokenType = 12;
pub const kExprLexNot: LexExprTokenType = 13;
pub const kExprLexNumber: LexExprTokenType = 14;
pub const kExprLexSingleQuotedString: LexExprTokenType = 15;
pub const kExprLexDoubleQuotedString: LexExprTokenType = 16;
pub const kExprLexOption: LexExprTokenType = 17;
pub const kExprLexRegister: LexExprTokenType = 18;
pub const kExprLexEnv: LexExprTokenType = 19;
pub const kExprLexPlainIdentifier: LexExprTokenType = 20;
pub const kExprLexBracket: LexExprTokenType = 21;
pub const kExprLexFigureBrace: LexExprTokenType = 22;
pub const kExprLexParenthesis: LexExprTokenType = 23;
pub const kExprLexComma: LexExprTokenType = 24;
pub const kExprLexArrow: LexExprTokenType = 25;
pub const kExprLexAssignment: LexExprTokenType = 26;

pub type ExprComparisonType = c_int;
pub const kExprCmpEqual: ExprComparisonType = 0;
pub const kExprCmpMatches: ExprComparisonType = 1;
pub const kExprCmpGreater: ExprComparisonType = 2;
pub const kExprCmpGreaterOrEqual: ExprComparisonType = 3;
pub const kExprCmpIdentical: ExprComparisonType = 4;

pub type ExprOptScope = c_int;
pub const kExprOptScopeUnspecified: ExprOptScope = 0;

pub type ExprAssignmentType = c_int;
pub const kExprAsgnPlain: ExprAssignmentType = 0;
pub const kExprAsgnAdd: ExprAssignmentType = 1;
pub const kExprAsgnSubtract: ExprAssignmentType = 2;
pub const kExprAsgnConcat: ExprAssignmentType = 3;

pub type ExprVarScope = c_int;
pub const kExprVarScopeMissing: ExprVarScope = 0;

pub type LexExprMulType = c_int;
pub const kExprLexMulMul: LexExprMulType = 0;
pub const kExprLexMulDiv: LexExprMulType = 1;
pub const kExprLexMulMod: LexExprMulType = 2;

pub const kELFlagPeek: c_int = 1 << 0;
pub const kELFlagForbidScope: c_int = 1 << 1;
pub const kELFlagAllowFloat: c_int = 1 << 2;
pub const kELFlagIsNotCmp: c_int = 1 << 3;
pub const kELFlagForbidEOC: c_int = 1 << 4;

pub type ExprASTNodeType = c_int;
pub const kExprNodeMissing: ExprASTNodeType = 0;
pub const kExprNodeOpMissing: ExprASTNodeType = 1;
pub const kExprNodeTernary: ExprASTNodeType = 2;
pub const kExprNodeTernaryValue: ExprASTNodeType = 3;
pub const kExprNodeRegister: ExprASTNodeType = 4;
pub const kExprNodeSubscript: ExprASTNodeType = 5;
pub const kExprNodeListLiteral: ExprASTNodeType = 6;
pub const kExprNodeUnaryPlus: ExprASTNodeType = 7;
pub const kExprNodeBinaryPlus: ExprASTNodeType = 8;
pub const kExprNodeNested: ExprASTNodeType = 9;
pub const kExprNodeCall: ExprASTNodeType = 10;
pub const kExprNodePlainIdentifier: ExprASTNodeType = 11;
pub const kExprNodePlainKey: ExprASTNodeType = 12;
pub const kExprNodeComplexIdentifier: ExprASTNodeType = 13;
pub const kExprNodeUnknownFigure: ExprASTNodeType = 14;
pub const kExprNodeLambda: ExprASTNodeType = 15;
pub const kExprNodeDictLiteral: ExprASTNodeType = 16;
pub const kExprNodeCurlyBracesIdentifier: ExprASTNodeType = 17;
pub const kExprNodeComma: ExprASTNodeType = 18;
pub const kExprNodeColon: ExprASTNodeType = 19;
pub const kExprNodeArrow: ExprASTNodeType = 20;
pub const kExprNodeComparison: ExprASTNodeType = 21;
pub const kExprNodeConcat: ExprASTNodeType = 22;
pub const kExprNodeConcatOrSubscript: ExprASTNodeType = 23;
pub const kExprNodeInteger: ExprASTNodeType = 24;
pub const kExprNodeFloat: ExprASTNodeType = 25;
pub const kExprNodeSingleQuotedString: ExprASTNodeType = 26;
pub const kExprNodeDoubleQuotedString: ExprASTNodeType = 27;
pub const kExprNodeOr: ExprASTNodeType = 28;
pub const kExprNodeAnd: ExprASTNodeType = 29;
pub const kExprNodeUnaryMinus: ExprASTNodeType = 30;
pub const kExprNodeBinaryMinus: ExprASTNodeType = 31;
pub const kExprNodeNot: ExprASTNodeType = 32;
pub const kExprNodeMultiplication: ExprASTNodeType = 33;
pub const kExprNodeDivision: ExprASTNodeType = 34;
pub const kExprNodeMod: ExprASTNodeType = 35;
pub const kExprNodeOption: ExprASTNodeType = 36;
pub const kExprNodeEnvironment: ExprASTNodeType = 37;
pub const kExprNodeAssignment: ExprASTNodeType = 38;

pub const kExprFlagsMulti: c_int = 1 << 0;
pub const kExprFlagsDisallowEOC: c_int = 1 << 1;
pub const kExprFlagsParseLet: c_int = 1 << 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenCmp {
    pub r#type: ExprComparisonType,
    pub ccs: ExprCaseCompareStrategy,
    pub inv: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenMul {
    pub r#type: LexExprMulType,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenBrc {
    pub closing: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenReg {
    pub name: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenStr {
    pub closed: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenOpt {
    pub name: *const c_char,
    pub len: usize,
    pub scope: ExprOptScope,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenVar {
    pub scope: ExprVarScope,
    pub autoload: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenErr {
    pub r#type: LexExprTokenType,
    pub msg: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenNumVal {
    pub floating: float_T,
    pub integer: uvarnumber_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenNum {
    pub val: LexExprTokenNumVal,
    pub base: u8,
    pub is_float: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenAss {
    pub r#type: ExprAssignmentType,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprTokenData {
    pub cmp: LexExprTokenCmp,
    pub mul: LexExprTokenMul,
    pub brc: LexExprTokenBrc,
    pub reg: LexExprTokenReg,
    pub str: LexExprTokenStr,
    pub opt: LexExprTokenOpt,
    pub var: LexExprTokenVar,
    pub err: LexExprTokenErr,
    pub num: LexExprTokenNum,
    pub ass: LexExprTokenAss,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LexExprToken {
    pub start: ParserPosition,
    pub len: usize,
    pub r#type: LexExprTokenType,
    pub data: LexExprTokenData,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeReg {
    pub name: c_int,
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeVar {
    pub scope: ExprVarScope,
    pub ident: *const c_char,
    pub ident_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeTer {
    pub got_colon: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeCmp {
    pub r#type: ExprComparisonType,
    pub ccs: ExprCaseCompareStrategy,
    pub inv: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeNum {
    pub value: uvarnumber_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeFlt {
    pub value: float_T,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeStr {
    pub value: *mut c_char,
    pub size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeOpt {
    pub ident: *const c_char,
    pub ident_len: usize,
    pub scope: ExprOptScope,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeEnv {
    pub ident: *const c_char,
    pub ident_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeAss {
    pub r#type: ExprAssignmentType,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNodeData {
    pub reg: ExprASTNodeReg,
    pub fig: ExprASTNodeFig,
    pub var: ExprASTNodeVar,
    pub ter: ExprASTNodeTer,
    pub cmp: ExprASTNodeCmp,
    pub num: ExprASTNodeNum,
    pub flt: ExprASTNodeFlt,
    pub str: ExprASTNodeStr,
    pub opt: ExprASTNodeOpt,
    pub env: ExprASTNodeEnv,
    pub ass: ExprASTNodeAss,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTNode {
    pub r#type: ExprASTNodeType,
    pub children: *mut ExprASTNode,
    pub next: *mut ExprASTNode,
    pub start: ParserPosition,
    pub len: usize,
    pub data: ExprASTNodeData,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprASTError {
    pub msg: *const c_char,
    pub arg: *const c_char,
    pub arg_len: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExprAST {
    pub err: ExprASTError,
    pub root: *mut ExprASTNode,
}

type ExprASTStack = Vec<*mut *mut ExprASTNode>;

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

#[repr(transparent)]
pub struct SyncCStrArray<const N: usize>(pub [*const c_char; N]);
unsafe impl<const N: usize> Sync for SyncCStrArray<N> {}

pub static eltkn_cmp_type_tab: SyncCStrArray<5> = SyncCStrArray([
    c"Equal".as_ptr(),
    c"Matches".as_ptr(),
    c"Greater".as_ptr(),
    c"GreaterOrEqual".as_ptr(),
    c"Identical".as_ptr(),
]);

pub static expr_asgn_type_tab: SyncCStrArray<4> = SyncCStrArray([
    c"Plain".as_ptr(),
    c"Add".as_ptr(),
    c"Subtract".as_ptr(),
    c"Concat".as_ptr(),
]);

const fn build_ccs_tab() -> [*const c_char; 64] {
    let mut tab = [std::ptr::null(); 64];
    tab[kCCStrategyUseOption as usize] = c"UseOption".as_ptr();
    tab[kCCStrategyMatchCase as usize] = c"MatchCase".as_ptr();
    tab[kCCStrategyIgnoreCase as usize] = c"IgnoreCase".as_ptr();
    tab
}

pub static ccs_tab: SyncCStrArray<64> = SyncCStrArray(build_ccs_tab());

pub static east_node_type_tab: SyncCStrArray<39> = SyncCStrArray([
    c"Missing".as_ptr(),
    c"OpMissing".as_ptr(),
    c"Ternary".as_ptr(),
    c"TernaryValue".as_ptr(),
    c"Register".as_ptr(),
    c"Subscript".as_ptr(),
    c"ListLiteral".as_ptr(),
    c"UnaryPlus".as_ptr(),
    c"BinaryPlus".as_ptr(),
    c"Nested".as_ptr(),
    c"Call".as_ptr(),
    c"PlainIdentifier".as_ptr(),
    c"PlainKey".as_ptr(),
    c"ComplexIdentifier".as_ptr(),
    c"UnknownFigure".as_ptr(),
    c"Lambda".as_ptr(),
    c"DictLiteral".as_ptr(),
    c"CurlyBracesIdentifier".as_ptr(),
    c"Comma".as_ptr(),
    c"Colon".as_ptr(),
    c"Arrow".as_ptr(),
    c"Comparison".as_ptr(),
    c"Concat".as_ptr(),
    c"ConcatOrSubscript".as_ptr(),
    c"Integer".as_ptr(),
    c"Float".as_ptr(),
    c"SingleQuotedString".as_ptr(),
    c"DoubleQuotedString".as_ptr(),
    c"Or".as_ptr(),
    c"And".as_ptr(),
    c"UnaryMinus".as_ptr(),
    c"BinaryMinus".as_ptr(),
    c"Not".as_ptr(),
    c"Multiplication".as_ptr(),
    c"Division".as_ptr(),
    c"Mod".as_ptr(),
    c"Option".as_ptr(),
    c"Environment".as_ptr(),
    c"Assignment".as_ptr(),
]);

pub static node_maxchildren: [u8; 39] = [
    0, // kExprNodeMissing
    2, // kExprNodeOpMissing
    2, // kExprNodeTernary
    2, // kExprNodeTernaryValue
    0, // kExprNodeRegister
    2, // kExprNodeSubscript
    1, // kExprNodeListLiteral
    1, // kExprNodeUnaryPlus
    2, // kExprNodeBinaryPlus
    1, // kExprNodeNested
    2, // kExprNodeCall
    0, // kExprNodePlainIdentifier
    0, // kExprNodePlainKey
    2, // kExprNodeComplexIdentifier
    1, // kExprNodeUnknownFigure
    2, // kExprNodeLambda
    1, // kExprNodeDictLiteral
    1, // kExprNodeCurlyBracesIdentifier
    2, // kExprNodeComma
    2, // kExprNodeColon
    2, // kExprNodeArrow
    2, // kExprNodeComparison
    2, // kExprNodeConcat
    2, // kExprNodeConcatOrSubscript
    0, // kExprNodeInteger
    0, // kExprNodeFloat
    0, // kExprNodeSingleQuotedString
    0, // kExprNodeDoubleQuotedString
    2, // kExprNodeOr
    2, // kExprNodeAnd
    1, // kExprNodeUnaryMinus
    2, // kExprNodeBinaryMinus
    1, // kExprNodeNot
    2, // kExprNodeMultiplication
    2, // kExprNodeDivision
    2, // kExprNodeMod
    0, // kExprNodeOption
    0, // kExprNodeEnvironment
    2, // kExprNodeAssignment
];

const NODE_PROPS: [(ExprOpLvl, ExprOpAssociativity); 39] = [
    (ExprOpLvl::Invalid, ExprOpAssociativity::No), // kExprNodeMissing
    (ExprOpLvl::Multiplication, ExprOpAssociativity::No), // kExprNodeOpMissing
    (ExprOpLvl::Ternary, ExprOpAssociativity::Right), // kExprNodeTernary
    (ExprOpLvl::TernaryValue, ExprOpAssociativity::Right), // kExprNodeTernaryValue
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeRegister
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // kExprNodeSubscript
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // kExprNodeListLiteral
    (ExprOpLvl::Unary, ExprOpAssociativity::No),   // kExprNodeUnaryPlus
    (ExprOpLvl::Addition, ExprOpAssociativity::Left), // kExprNodeBinaryPlus
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // kExprNodeNested
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // kExprNodeCall
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodePlainIdentifier
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodePlainKey
    (ExprOpLvl::Value, ExprOpAssociativity::Left), // kExprNodeComplexIdentifier
    (ExprOpLvl::Parens, ExprOpAssociativity::Left), // kExprNodeUnknownFigure
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // kExprNodeLambda
    (ExprOpLvl::Parens, ExprOpAssociativity::No),  // kExprNodeDictLiteral
    (ExprOpLvl::ComplexIdentifier, ExprOpAssociativity::Left), // kExprNodeCurlyBracesIdentifier
    (ExprOpLvl::Comma, ExprOpAssociativity::Right), // kExprNodeComma
    (ExprOpLvl::Colon, ExprOpAssociativity::No),   // kExprNodeColon
    (ExprOpLvl::Arrow, ExprOpAssociativity::No),   // kExprNodeArrow
    (ExprOpLvl::Comparison, ExprOpAssociativity::Right), // kExprNodeComparison
    (ExprOpLvl::Addition, ExprOpAssociativity::Left), // kExprNodeConcat
    (ExprOpLvl::Subscript, ExprOpAssociativity::Left), // kExprNodeConcatOrSubscript
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeInteger
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeFloat
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeSingleQuotedString
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeDoubleQuotedString
    (ExprOpLvl::Or, ExprOpAssociativity::Left),    // kExprNodeOr
    (ExprOpLvl::And, ExprOpAssociativity::Left),   // kExprNodeAnd
    (ExprOpLvl::Unary, ExprOpAssociativity::No),   // kExprNodeUnaryMinus
    (ExprOpLvl::Addition, ExprOpAssociativity::Left), // kExprNodeBinaryMinus
    (ExprOpLvl::Unary, ExprOpAssociativity::No),   // kExprNodeNot
    (ExprOpLvl::Multiplication, ExprOpAssociativity::Left), // kExprNodeMultiplication
    (ExprOpLvl::Multiplication, ExprOpAssociativity::Left), // kExprNodeDivision
    (ExprOpLvl::Multiplication, ExprOpAssociativity::Left), // kExprNodeMod
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeOption
    (ExprOpLvl::Value, ExprOpAssociativity::No),   // kExprNodeEnvironment
    (ExprOpLvl::Assignment, ExprOpAssociativity::Left), // kExprNodeAssignment
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
unsafe fn viml_parser_advance(pstate: *mut ParserState, len: usize) {
    unsafe {
        let pstate = &mut *pstate;
        debug_assert_eq!(pstate.pos.line, pstate.reader.lines.len() - 1);
        let pline = *pstate.reader.lines.last().unwrap();
        if pstate.pos.col + len >= pline.size {
            pstate.pos.line += 1;
            pstate.pos.col = 0;
        } else {
            pstate.pos.col += len;
        }
    }
}

#[inline]
unsafe fn viml_parser_highlight(
    pstate: *mut ParserState,
    start: ParserPosition,
    len: usize,
    group: *const c_char,
) {
    unsafe {
        let pstate = &mut *pstate;
        if pstate.colors.is_null() || len == 0 {
            return;
        }
        let colors = &mut *pstate.colors;
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
}

#[inline]
fn scale_number(
    num: float_T,
    base: u8,
    exponent: uvarnumber_T,
    exponent_negative: bool,
) -> float_T {
    if num == 0.0 || exponent == 0 {
        return num;
    }
    debug_assert_ne!(base, 0);
    let mut exp = exponent;
    let mut p_base = float_T::from(base);
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
unsafe fn get_ccs(ret: &mut LexExprToken, bytes: &[u8]) {
    if ret.len < bytes.len() && (bytes[ret.len] == b'?' || bytes[ret.len] == b'#') {
        ret.data.cmp.ccs = c_int::from(bytes[ret.len]);
        ret.len += 1;
    } else {
        ret.data.cmp.ccs = kCCStrategyUseOption;
    }
}

/// Get next token for the Vimscript expression input (`viml_pexpr_next_token`).
pub unsafe fn viml_pexpr_next_token(pstate: *mut ParserState, flags: c_int) -> LexExprToken {
    unsafe {
        let mut ret: LexExprToken = std::mem::zeroed();
        ret.r#type = kExprLexInvalid;
        ret.start = (*pstate).pos;

        let mut pline = ParserLine {
            data: std::ptr::null(),
            size: 0,
            allocated: false,
        };
        if !viml_parser_get_remaining_line(pstate, &mut pline) {
            ret.r#type = kExprLexEOC;
            return ret;
        }
        if pline.size == 0 {
            ret.len = 0;
            ret.r#type = kExprLexEOC;
            if (flags & kELFlagPeek) == 0 {
                viml_parser_advance(pstate, ret.len);
            }
            return ret;
        }
        let bytes = std::slice::from_raw_parts(pline.data.cast::<u8>(), pline.size);
        ret.len = 1;
        let schar = bytes[0];

        match schar {
            b'(' | b')' => {
                ret.r#type = kExprLexParenthesis;
                ret.data.brc.closing = schar == b')';
            }
            b'[' | b']' => {
                ret.r#type = kExprLexBracket;
                ret.data.brc.closing = schar == b']';
            }
            b'{' | b'}' => {
                ret.r#type = kExprLexFigureBrace;
                ret.data.brc.closing = schar == b'}';
            }
            b'?' => {
                ret.r#type = kExprLexQuestion;
            }
            b':' => {
                ret.r#type = kExprLexColon;
            }
            b',' => {
                ret.r#type = kExprLexComma;
            }
            b'*' => {
                ret.r#type = kExprLexMultiplication;
                ret.data.mul.r#type = kExprLexMulMul;
            }
            b'/' => {
                ret.r#type = kExprLexMultiplication;
                ret.data.mul.r#type = kExprLexMulDiv;
            }
            b'%' => {
                ret.r#type = kExprLexMultiplication;
                ret.data.mul.r#type = kExprLexMulMod;
            }
            b' ' | TAB => {
                ret.r#type = kExprLexSpacing;
                while ret.len < bytes.len() && ascii_iswhite(bytes[ret.len]) {
                    ret.len += 1;
                }
            }
            Ctrl_A..=Ctrl_H | Ctrl_K..=Ctrl_Z => {
                ret.r#type = kExprLexInvalid;
                while ret.len < bytes.len() && (bytes[ret.len] as i8) < (b' ' as i8) {
                    ret.len += 1;
                }
                ret.data.err.r#type = kExprLexSpacing;
                ret.data.err.msg =
                    c"E15: Invalid control character present in input: %.*s".as_ptr();
            }
            b'0'..=b'9' => {
                ret.data.num.is_float = false;
                ret.data.num.base = 10;
                let mut frac_start = 0usize;
                let mut exp_start = 0usize;
                let mut frac_end = 0usize;
                let mut exp_negative = false;
                ret.r#type = kExprLexNumber;
                while ret.len < bytes.len() && ascii_isdigit(bytes[ret.len]) {
                    ret.len += 1;
                }
                if (flags & kELFlagAllowFloat) != 0 {
                    let non_float_ret = ret;
                    if bytes.len() > ret.len + 1
                        && bytes[ret.len] == b'.'
                        && ascii_isdigit(bytes[ret.len + 1])
                    {
                        ret.len += 1;
                        frac_start = ret.len;
                        frac_end = ret.len;
                        ret.data.num.is_float = true;
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
                            ret.r#type = kExprLexNumber;
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
                if ret.data.num.is_float {
                    let mut significand_part: float_T = 0.0;
                    let mut exp_part: uvarnumber_T = 0;
                    let frac_size = (frac_end - frac_start) as uvarnumber_T;
                    for (i, &b) in bytes[..frac_end].iter().enumerate() {
                        if i == frac_start - 1 {
                            continue;
                        }
                        significand_part = significand_part * 10.0 + float_T::from(b - b'0');
                    }
                    if exp_start != 0 {
                        vim_str2nr(
                            pline.data.add(exp_start),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                            &mut exp_part,
                            (ret.len - exp_start) as c_int,
                            false,
                            std::ptr::null_mut(),
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
                    ret.data.num.val.floating =
                        scale_number(significand_part, 10, exp_part, exp_negative);
                } else {
                    let mut len: c_int = 0;
                    let mut prep: c_int = 0;
                    let mut int_val: uvarnumber_T = 0;
                    vim_str2nr(
                        pline.data,
                        &mut prep,
                        &mut len,
                        STR2NR_ALL,
                        std::ptr::null_mut(),
                        &mut int_val,
                        pline.size as c_int,
                        false,
                        std::ptr::null_mut(),
                    );
                    ret.data.num.val.integer = int_val;
                    ret.len = len as usize;
                    ret.data.num.base = match prep as u8 {
                        0 => 10,
                        b'0' => 8,
                        b'x' | b'X' => 16,
                        b'b' | b'B' => 2,
                        _ => 10,
                    };
                }
            }
            b'$' => {
                ret.r#type = kExprLexEnv;
                while ret.len < bytes.len() && ascii_isident(bytes[ret.len]) {
                    ret.len += 1;
                }
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                ret.data.var.scope = 0;
                ret.data.var.autoload = false;
                ret.r#type = kExprLexPlainIdentifier;
                while ret.len < bytes.len() && ascii_isident(bytes[ret.len]) {
                    ret.len += 1;
                }
                if (flags & kELFlagIsNotCmp) == 0
                    && ((ret.len == 2 && &bytes[..2] == b"is")
                        || (ret.len == 5 && &bytes[..5] == b"isnot"))
                {
                    ret.r#type = kExprLexComparison;
                    ret.data.cmp.r#type = kExprCmpIdentical;
                    ret.data.cmp.inv = ret.len == 5;
                    get_ccs(&mut ret, bytes);
                } else if ret.len == 1
                    && bytes.len() > 1
                    && is_var_scope(schar)
                    && bytes[ret.len] == b':'
                    && (flags & kELFlagForbidScope) == 0
                {
                    ret.len += 1;
                    ret.data.var.scope = c_int::from(schar);
                    ret.r#type = kExprLexPlainIdentifier;
                    while ret.len < bytes.len()
                        && (ascii_isident(bytes[ret.len]) || bytes[ret.len] == AUTOLOAD_CHAR)
                    {
                        ret.len += 1;
                    }
                    ret.data.var.autoload = bytes[2..ret.len].contains(&AUTOLOAD_CHAR);
                } else if bytes.len() > ret.len && bytes[ret.len] == AUTOLOAD_CHAR {
                    ret.data.var.autoload = true;
                    ret.r#type = kExprLexPlainIdentifier;
                    while ret.len < bytes.len()
                        && (ascii_isident(bytes[ret.len]) || bytes[ret.len] == AUTOLOAD_CHAR)
                    {
                        ret.len += 1;
                    }
                }
            }
            b'&' => {
                if bytes.len() > 1 && bytes[1] == b'&' {
                    ret.r#type = kExprLexAnd;
                    ret.len += 1;
                } else if bytes.len() == 1 || !ascii_isalpha(bytes[1]) {
                    ret.r#type = kExprLexInvalid;
                    ret.data.err.r#type = kExprLexOption;
                    ret.data.err.msg = c"E112: Option name missing: %.*s".as_ptr();
                } else {
                    ret.r#type = kExprLexOption;
                    let p_offset: usize =
                        if bytes.len() > 2 && bytes[2] == b':' && is_opt_scope(bytes[1]) {
                            ret.len += 2;
                            ret.data.opt.scope = c_int::from(bytes[1]);
                            ret.data.opt.name = pline.data.add(3);
                            3
                        } else {
                            ret.data.opt.scope = kExprOptScopeUnspecified;
                            ret.data.opt.name = pline.data.add(1);
                            1
                        };
                    if bytes.len() - p_offset >= 4
                        && bytes[p_offset] == b't'
                        && bytes[p_offset + 1] == b'_'
                    {
                        ret.data.opt.len = 4;
                        ret.len += 4;
                    } else {
                        let mut p_idx = p_offset;
                        while p_idx < bytes.len() && ascii_isalpha(bytes[p_idx]) {
                            p_idx += 1;
                        }
                        ret.data.opt.len = p_idx - p_offset;
                        if ret.data.opt.len == 0 {
                            ret.r#type = kExprLexInvalid;
                            ret.data.err.r#type = kExprLexOption;
                            ret.data.err.msg = c"E112: Option name missing: %.*s".as_ptr();
                        } else {
                            ret.len += ret.data.opt.len;
                        }
                    }
                }
            }
            b'@' => {
                ret.r#type = kExprLexRegister;
                if bytes.len() > 1 {
                    ret.len += 1;
                    ret.data.reg.name = c_int::from(bytes[1]);
                } else {
                    ret.data.reg.name = -1;
                }
            }
            b'\'' => {
                ret.r#type = kExprLexSingleQuotedString;
                ret.data.str.closed = false;
                while ret.len < bytes.len() && !ret.data.str.closed {
                    if bytes[ret.len] == b'\'' {
                        if ret.len + 1 < bytes.len() && bytes[ret.len + 1] == b'\'' {
                            ret.len += 1;
                        } else {
                            ret.data.str.closed = true;
                        }
                    }
                    ret.len += 1;
                }
            }
            b'"' => {
                ret.r#type = kExprLexDoubleQuotedString;
                ret.data.str.closed = false;
                while ret.len < bytes.len() && !ret.data.str.closed {
                    if bytes[ret.len] == b'\\' {
                        if ret.len + 1 < bytes.len() {
                            ret.len += 1;
                        }
                    } else if bytes[ret.len] == b'"' {
                        ret.data.str.closed = true;
                    }
                    ret.len += 1;
                }
            }
            b'!' | b'=' => {
                if bytes.len() == 1 {
                    ret.r#type = if schar == b'!' {
                        kExprLexNot
                    } else {
                        kExprLexAssignment
                    };
                    ret.data.ass.r#type = kExprAsgnPlain;
                } else {
                    ret.r#type = kExprLexComparison;
                    ret.data.cmp.inv = schar == b'!';
                    if bytes[1] == b'=' {
                        ret.data.cmp.r#type = kExprCmpEqual;
                        ret.len += 1;
                    } else if bytes[1] == b'~' {
                        ret.data.cmp.r#type = kExprCmpMatches;
                        ret.len += 1;
                    } else if schar == b'!' {
                        ret.r#type = kExprLexNot;
                    } else {
                        ret.r#type = kExprLexAssignment;
                        ret.data.ass.r#type = kExprAsgnPlain;
                    }
                    get_ccs(&mut ret, bytes);
                }
            }
            b'>' | b'<' => {
                ret.r#type = kExprLexComparison;
                let haseqsign = bytes.len() > 1 && bytes[1] == b'=';
                if haseqsign {
                    ret.len += 1;
                }
                get_ccs(&mut ret, bytes);
                ret.data.cmp.inv = schar == b'<';
                ret.data.cmp.r#type = if ret.data.cmp.inv ^ haseqsign {
                    kExprCmpGreaterOrEqual
                } else {
                    kExprCmpGreater
                };
            }
            b'-' => {
                if bytes.len() > 1 && bytes[1] == b'>' {
                    ret.len += 1;
                    ret.r#type = kExprLexArrow;
                } else if bytes.len() > 1 && bytes[1] == b'=' {
                    ret.len += 1;
                    ret.r#type = kExprLexAssignment;
                    ret.data.ass.r#type = kExprAsgnSubtract;
                } else {
                    ret.r#type = kExprLexMinus;
                }
            }
            b'+' => {
                if bytes.len() > 1 && bytes[1] == b'=' {
                    ret.len += 1;
                    ret.r#type = kExprLexAssignment;
                    ret.data.ass.r#type = kExprAsgnAdd;
                } else {
                    ret.r#type = kExprLexPlus;
                }
            }
            b'.' => {
                if bytes.len() > 1 && bytes[1] == b'=' {
                    ret.len += 1;
                    ret.r#type = kExprLexAssignment;
                    ret.data.ass.r#type = kExprAsgnConcat;
                } else {
                    ret.r#type = kExprLexDot;
                }
            }
            NUL | NL => {
                if (flags & kELFlagForbidEOC) != 0 {
                    ret.r#type = kExprLexInvalid;
                    ret.data.err.msg = c"E15: Unexpected EOC character: %.*s".as_ptr();
                    ret.data.err.r#type = kExprLexSpacing;
                } else {
                    ret.r#type = kExprLexEOC;
                }
            }
            b'|' => {
                if bytes.len() >= 2 && bytes[ret.len] == b'|' {
                    ret.len += 1;
                    ret.r#type = kExprLexOr;
                } else if (flags & kELFlagForbidEOC) != 0 {
                    ret.r#type = kExprLexInvalid;
                    ret.data.err.msg = c"E15: Unexpected EOC character: %.*s".as_ptr();
                    ret.data.err.r#type = kExprLexOr;
                } else {
                    ret.r#type = kExprLexEOC;
                }
            }
            _ => {
                ret.len = utfc_ptr2len_len(pline.data, pline.size as c_int) as usize;
                ret.r#type = kExprLexInvalid;
                ret.data.err.r#type = kExprLexPlainIdentifier;
                ret.data.err.msg = c"E15: Unidentified character: %.*s".as_ptr();
            }
        }

        if (flags & kELFlagPeek) == 0 {
            viml_parser_advance(pstate, ret.len);
        }
        ret
    }
}

/// Free memory occupied by AST (`viml_pexpr_free_ast`).
pub unsafe fn viml_pexpr_free_ast(mut ast: ExprAST) {
    unsafe {
        let mut ast_stack: ExprASTStack = vec![&mut ast.root];
        while let Some(&cur_node) = ast_stack.last() {
            for &prev in &ast_stack[..ast_stack.len() - 1] {
                debug_assert_ne!(*prev, *cur_node);
            }
            if (*cur_node).is_null() {
                debug_assert_eq!(ast_stack.len(), 1);
                ast_stack.pop();
            } else if !(**cur_node).children.is_null() {
                let maxchildren = node_maxchildren[(**cur_node).r#type as usize];
                debug_assert!(maxchildren > 0 && maxchildren <= 2);
                ast_stack.push(&mut (**cur_node).children);
            } else if !(**cur_node).next.is_null() {
                ast_stack.push(&mut (**cur_node).next);
            } else {
                ast_stack.pop();
                let node_type = (**cur_node).r#type;
                if node_type == kExprNodeDoubleQuotedString
                    || node_type == kExprNodeSingleQuotedString
                {
                    xfree((**cur_node).data.str.value.cast());
                }
                drop(Box::from_raw(*cur_node));
                *cur_node = std::ptr::null_mut();
            }
        }
    }
}

#[inline]
unsafe fn viml_pexpr_new_node(node_type: ExprASTNodeType) -> *mut ExprASTNode {
    unsafe {
        let mut ret = Box::new(std::mem::zeroed::<ExprASTNode>());
        ret.r#type = node_type;
        ret.children = std::ptr::null_mut();
        ret.next = std::ptr::null_mut();
        Box::into_raw(ret)
    }
}

#[inline]
unsafe fn new_node_with_cur_pos(
    node_type: ExprASTNodeType,
    cur_token: LexExprToken,
    prev_token: LexExprToken,
) -> *mut ExprASTNode {
    unsafe {
        let node = viml_pexpr_new_node(node_type);
        (*node).start = cur_token.start;
        (*node).len = cur_token.len;
        if prev_token.r#type == kExprLexSpacing {
            (*node).start = prev_token.start;
            (*node).len += prev_token.len;
        }
        node
    }
}

#[inline]
fn node_lvl(node: ExprASTNode) -> ExprOpLvl {
    NODE_PROPS[node.r#type as usize].0
}

#[inline]
fn node_ass(node: ExprASTNode) -> ExprOpAssociativity {
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
unsafe fn east_set_error(
    pstate: *const ParserState,
    ret_ast_err: *mut ExprASTError,
    msg: *const c_char,
    start: ParserPosition,
) {
    unsafe {
        if !(*ret_ast_err).msg.is_null() {
            return;
        }
        let pline = (*pstate).reader.lines.as_slice()[start.line];
        (*ret_ast_err).msg = msg;
        (*ret_ast_err).arg_len = (pline.size - start.col) as c_int;
        (*ret_ast_err).arg = if pline.data.is_null() {
            std::ptr::null()
        } else {
            pline.data.add(start.col)
        };
    }
}

unsafe fn viml_pexpr_handle_bop(
    pstate: *const ParserState,
    ast_stack: *mut ExprASTStack,
    bop_node: *mut ExprASTNode,
    want_node_p: *mut ExprASTWantedNode,
    ast_err: *mut ExprASTError,
) -> bool {
    unsafe {
        let mut ret = true;
        let mut top_node_p: *mut *mut ExprASTNode = std::ptr::null_mut();
        let mut top_node: *mut ExprASTNode = std::ptr::null_mut();
        let mut top_node_lvl = ExprOpLvl::Invalid;
        let mut top_node_ass = ExprOpAssociativity::No;
        debug_assert!(!(*ast_stack).is_empty());
        let bop_node_lvl =
            if (*bop_node).r#type == kExprNodeCall || (*bop_node).r#type == kExprNodeSubscript {
                ExprOpLvl::Subscript
            } else {
                node_lvl(*bop_node)
            };
        loop {
            let new_top_node_p = *(*ast_stack).last().unwrap();
            let new_top_node = *new_top_node_p;
            debug_assert!(!new_top_node.is_null());
            let new_top_node_lvl = node_lvl(*new_top_node);
            let new_top_node_ass = node_ass(*new_top_node);
            if !top_node_p.is_null()
                && (bop_node_lvl > new_top_node_lvl
                    || (bop_node_lvl == new_top_node_lvl
                        && new_top_node_ass == ExprOpAssociativity::No))
            {
                break;
            }
            (*ast_stack).pop();
            top_node_p = new_top_node_p;
            top_node = new_top_node;
            top_node_lvl = new_top_node_lvl;
            top_node_ass = new_top_node_ass;
            if bop_node_lvl == top_node_lvl && top_node_ass == ExprOpAssociativity::Right {
                break;
            }
            if (*ast_stack).is_empty() {
                break;
            }
        }
        if top_node_ass == ExprOpAssociativity::Left || top_node_lvl != bop_node_lvl {
            *top_node_p = bop_node;
            (*bop_node).children = top_node;
            debug_assert!((*(*bop_node).children).next.is_null());
            (*ast_stack).push(top_node_p);
            (*ast_stack).push(&mut (*(*bop_node).children).next);
        } else {
            debug_assert!(
                top_node_lvl == bop_node_lvl && top_node_ass == ExprOpAssociativity::Right
            );
            debug_assert!(
                !(*top_node).children.is_null() && !(*(*top_node).children).next.is_null()
            );
            (*bop_node).children = (*(*top_node).children).next;
            (*(*top_node).children).next = bop_node;
            debug_assert!((*(*bop_node).children).next.is_null());
            (*ast_stack).push(top_node_p);
            (*ast_stack).push(&mut (*(*top_node).children).next);
            (*ast_stack).push(&mut (*(*bop_node).children).next);
            if (*bop_node).r#type == kExprNodeComparison {
                east_set_error(
                    pstate,
                    ast_err,
                    c"E15: Operator is not associative: %.*s".as_ptr(),
                    (*bop_node).start,
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
        if $is_invalid {
            $invalid.as_ptr()
        } else {
            $valid.as_ptr()
        }
    };
}

unsafe fn parse_quoted_string(
    pstate: *mut ParserState,
    node: *mut ExprASTNode,
    token: LexExprToken,
    _ast_stack: *const ExprASTStack,
    is_invalid: bool,
) {
    unsafe {
        let pline = (*pstate).reader.lines.as_slice()[token.start.line];
        let s = pline.data.add(token.start.col);
        let closed_len = usize::from(token.data.str.closed);
        let e = s.add(token.len - closed_len);
        let mut p = s.add(1);
        let is_double = token.r#type == kExprLexDoubleQuotedString;
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
                let slice = std::slice::from_raw_parts(p.cast::<u8>(), rem);
                let Some(pos) = slice.iter().position(|&b| b == b'\'') else {
                    break;
                };
                let chunk_e = p.add(pos);
                size -= 1;
                p = chunk_e.add(2);
                if !(*pstate).colors.is_null() {
                    shifts.push(StringShift {
                        start: token.start.col + (chunk_e as usize - s as usize),
                        orig_len: 2,
                        act_len: 1,
                        escape_not_known: false,
                    });
                }
            }
            (*node).data.str.size = size;
            if size == 0 {
                (*node).data.str.value = std::ptr::null_mut();
            } else {
                let mut v_p = xmallocz(size).cast::<c_char>();
                (*node).data.str.value = v_p;
                p = s.add(1);
                while (p as usize) < (e as usize) {
                    let rem = e as usize - p as usize;
                    let slice = std::slice::from_raw_parts(p.cast::<u8>(), rem);
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
                if *p == b'\\' as c_char && (p.add(1) as usize) < (e as usize) {
                    p = p.add(1);
                    if std::ptr::eq(p.add(1), e) {
                        size -= 1;
                        break;
                    }
                    match *p as u8 {
                        b'<' => {
                            size += 5;
                        }
                        b'x' | b'X' => {
                            size -= 1;
                            if ascii_isxdigit(*p.add(1) as u8) {
                                size -= 1;
                                if (p.add(2) as usize) < (e as usize)
                                    && ascii_isxdigit(*p.add(2) as u8)
                                {
                                    size -= 1;
                                }
                            }
                        }
                        b'u' | b'U' => {
                            let esc_start = p;
                            let mut n: usize = if *p as u8 == b'u' { 4 } else { 8 };
                            let mut nr: c_int = 0;
                            p = p.add(1);
                            while (p.add(1) as usize) < (e as usize)
                                && n != 0
                                && ascii_isxdigit(*p.add(1) as u8)
                            {
                                n -= 1;
                                p = p.add(1);
                                nr = (nr << 4) + hex2nr(c_int::from(*p as u8));
                            }
                            let esc_len = (p as usize) - (esc_start as usize - 1);
                            let char_len = utf_char2len(nr) as usize;
                            size = size.wrapping_sub(esc_len.wrapping_sub(char_len));
                            p = p.sub(1);
                        }
                        b'0'..=b'7' => {
                            size -= 1;
                            p = p.add(1);
                            if (*p as u8) >= b'0' && (*p as u8) <= b'7' {
                                size -= 1;
                                p = p.add(1);
                                if (p as usize) < (e as usize)
                                    && (*p as u8) >= b'0'
                                    && (*p as u8) <= b'7'
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
                (*node).data.str.value = std::ptr::null_mut();
                (*node).data.str.size = 0;
            } else {
                let mut v_p = xmalloc(size).cast::<c_char>();
                (*node).data.str.value = v_p;
                p = s.add(1);
                while (p as usize) < (e as usize) {
                    let rem = e as usize - p as usize;
                    let slice = std::slice::from_raw_parts(p.cast::<u8>(), rem);
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
                    match *p as u8 {
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
                            if (p.add(1) as usize) < (e as usize) && ascii_isxdigit(*p.add(1) as u8)
                            {
                                let is_hex = *p as u8 == b'x' || *p as u8 == b'X';
                                let mut n: usize = if is_hex {
                                    2
                                } else if *p as u8 == b'u' {
                                    4
                                } else {
                                    8
                                };
                                let mut nr: c_int = 0;
                                while (p.add(1) as usize) < (e as usize)
                                    && n != 0
                                    && ascii_isxdigit(*p.add(1) as u8)
                                {
                                    n -= 1;
                                    p = p.add(1);
                                    nr = (nr << 4) + hex2nr(c_int::from(*p as u8));
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
                            let mut ch = (*p as u8) - b'0';
                            p = p.add(1);
                            if (p as usize) < (e as usize)
                                && (*p as u8) >= b'0'
                                && (*p as u8) <= b'7'
                            {
                                ch = (ch << 3).wrapping_add(*p as u8 - b'0');
                                p = p.add(1);
                                if (p as usize) < (e as usize)
                                    && (*p as u8) >= b'0'
                                    && (*p as u8) <= b'7'
                                {
                                    ch = (ch << 3).wrapping_add(*p as u8 - b'0');
                                    p = p.add(1);
                                }
                            }
                            *v_p = ch as c_char;
                            v_p = v_p.add(1);
                        }
                        b'<' => {
                            let mut flags = FSK_KEYCODE | FSK_IN_STRING;
                            if *p.add(1) != b'*' as c_char {
                                flags |= FSK_SIMPLIFY;
                            }
                            let special_len = trans_special(
                                &mut p,
                                e as usize - p as usize,
                                v_p,
                                flags,
                                false,
                                std::ptr::null_mut(),
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
                    if !(*pstate).colors.is_null() {
                        shifts.push(StringShift {
                            start: token.start.col + (chunk_e as usize - s as usize),
                            orig_len: p as usize - chunk_e as usize,
                            act_len: v_p as usize - v_p_start as usize,
                            escape_not_known: is_unknown,
                        });
                    }
                }
                (*node).data.str.size = v_p as usize - (*node).data.str.value as usize;
            }
        }
        if !(*pstate).colors.is_null() {
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
        if token.data.str.closed {
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
    kELFlagForbidScope, // Operator
    kELFlagIsNotCmp,    // Value
];

const BASE_TO_PREFIX_LENGTH: [usize; 17] = [0, 0, 2, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2];

/// Parse one Vimscript expression (`viml_pexpr_parse`).
pub unsafe fn viml_pexpr_parse(pstate: *mut ParserState, flags: c_int) -> ExprAST {
    unsafe {
        let mut ast = ExprAST {
            err: ExprASTError {
                msg: std::ptr::null(),
                arg: std::ptr::null(),
                arg_len: 0,
            },
            root: std::ptr::null_mut(),
        };
        let mut ast_stack: ExprASTStack = vec![&mut ast.root];
        let mut want_node = ExprASTWantedNode::Value;
        let mut pt_stack: ExprASTParseTypeStack = vec![ExprASTParseType::Expr];
        if (flags & kExprFlagsParseLet) != 0 {
            pt_stack.push(ExprASTParseType::Assignment);
        }
        let mut prev_token: LexExprToken = std::mem::zeroed();
        prev_token.r#type = kExprLexMissing;
        let mut highlighted_prev_spacing = false;
        let mut lambda_node: *mut ExprASTNode = std::ptr::null_mut();
        let mut asgn_level: usize = 0;

        'parse_loop: loop {
            let is_concat_or_subscript = want_node == ExprASTWantedNode::Value
                && ast_stack.len() > 1
                && (**ast_stack[ast_stack.len() - 2]).r#type == kExprNodeConcatOrSubscript;
            let lexer_additional_flags = kELFlagPeek
                | if (flags & kExprFlagsDisallowEOC) != 0 {
                    kELFlagForbidEOC
                } else {
                    0
                }
                | if want_node == ExprASTWantedNode::Value
                    && (ast_stack.len() == 1
                        || ((**ast_stack[ast_stack.len() - 2]).r#type != kExprNodeConcat
                            && (**ast_stack[ast_stack.len() - 2]).r#type
                                != kExprNodeConcatOrSubscript))
                {
                    kELFlagAllowFloat
                } else {
                    0
                };
            let mut cur_token = viml_pexpr_next_token(
                pstate,
                WANT_NODE_TO_LEXER_FLAGS[want_node as usize] | lexer_additional_flags,
            );
            if cur_token.r#type == kExprLexEOC {
                break;
            }
            let mut tok_type = cur_token.r#type;
            let mut is_invalid = tok_type == kExprLexInvalid;

            'process_token: loop {
                cur_token = viml_pexpr_next_token(
                    pstate,
                    WANT_NODE_TO_LEXER_FLAGS[want_node as usize] | lexer_additional_flags,
                );
                if tok_type == kExprLexSpacing {
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
                    && prev_token.r#type == kExprLexSpacing
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
                let pline = (*pstate).reader.lines.as_slice()[cur_token.start.line];
                let top_node_p = *ast_stack.last().unwrap();
                debug_assert!(!ast_stack.is_empty());

                let node_is_key = is_concat_or_subscript
                    && (if cur_token.r#type == kExprLexPlainIdentifier {
                        !cur_token.data.var.autoload
                            && cur_token.data.var.scope == kExprVarScopeMissing
                    } else {
                        cur_token.r#type == kExprLexNumber
                    })
                    && prev_token.r#type != kExprLexSpacing;
                if is_concat_or_subscript && !node_is_key {
                    let prev_idx = ast_stack.len() - 2;
                    (**ast_stack[prev_idx]).r#type = kExprNodeConcat;
                }

                let is_single_assignment =
                    *pt_stack.last().unwrap() == ExprASTParseType::SingleAssignment;
                match *pt_stack.last().unwrap() {
                    ExprASTParseType::Expr => {}
                    ExprASTParseType::LambdaArguments => {
                        if (want_node == ExprASTWantedNode::Operator
                            && tok_type != kExprLexComma
                            && tok_type != kExprLexArrow)
                            || (want_node == ExprASTWantedNode::Value
                                && !(cur_token.r#type == kExprLexPlainIdentifier
                                    && cur_token.data.var.scope == kExprVarScopeMissing
                                    && !cur_token.data.var.autoload)
                                && tok_type != kExprLexArrow)
                        {
                            (*lambda_node).data.fig.type_guesses.allow_lambda = false;
                            if !(*lambda_node).children.is_null()
                                && (*(*lambda_node).children).r#type == kExprNodeComma
                            {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Expected lambda arguments list or arrow: %.*s".as_ptr(),
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
                            && tok_type != kExprLexBracket
                            && tok_type != kExprLexPlainIdentifier
                            && (tok_type != kExprLexFigureBrace || cur_token.data.brc.closing)
                            && !(node_is_key && tok_type == kExprLexNumber)
                            && tok_type != kExprLexEnv
                            && tok_type != kExprLexOption
                            && tok_type != kExprLexRegister
                        {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Expected value part of assignment lvalue: %.*s".as_ptr(),
                                cur_token.start,
                            );
                            pt_stack.pop();
                        } else if want_node == ExprASTWantedNode::Operator
                            && tok_type != kExprLexBracket
                            && (tok_type != kExprLexFigureBrace || cur_token.data.brc.closing)
                            && tok_type != kExprLexDot
                            && (tok_type != kExprLexComma || !is_single_assignment)
                            && tok_type != kExprLexAssignment
                            && !((tok_type == kExprLexPlainIdentifier
                                || (tok_type == kExprLexFigureBrace
                                    && !cur_token.data.brc.closing))
                                && prev_token.r#type != kExprLexSpacing)
                        {
                            if (flags & kExprFlagsMulti) != 0 && ast_stack.len() == 1 {
                                break 'parse_loop;
                            }
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Expected assignment operator or subscript: %.*s".as_ptr(),
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
                        if (flags & kExprFlagsMulti) != 0 && ast_stack.len() == 1 {
                            break 'parse_loop;
                        } else {
                            debug_assert!(!(*top_node_p).is_null());
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Missing operator: %.*s".as_ptr(),
                                cur_token.start,
                            );
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeOpMissing, cur_token, prev_token);
                            (*cur_node).len = 0;
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
                            *top_node_p =
                                new_node_with_cur_pos(kExprNodeMissing, cur_token, prev_token);
                            (**top_node_p).len = 0;
                            want_node = ExprASTWantedNode::Operator;
                        }
                    }};
                }

                match tok_type {
                    kExprLexMissing | kExprLexSpacing | kExprLexEOC => {
                        std::process::abort();
                    }
                    kExprLexInvalid => {
                        is_invalid = true;
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            cur_token.data.err.msg,
                            cur_token.start,
                        );
                        tok_type = cur_token.data.err.r#type;
                        continue 'process_token;
                    }
                    kExprLexRegister => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let cur_node =
                            new_node_with_cur_pos(kExprNodeRegister, cur_token, prev_token);
                        (*cur_node).data.reg.name = cur_token.data.reg.name;
                        *top_node_p = cur_node;
                        want_node = ExprASTWantedNode::Operator;
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimRegister", c"NvimInvalidRegister"),
                        );
                    }
                    kExprLexPlus => {
                        if want_node == ExprASTWantedNode::Value {
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeUnaryPlus, cur_token, prev_token);
                            *top_node_p = cur_node;
                            ast_stack.push(&mut (*cur_node).children);
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimUnaryPlus", c"NvimInvalidUnaryPlus"),
                            );
                        } else {
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeBinaryPlus, cur_token, prev_token);
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
                    kExprLexMinus => {
                        if want_node == ExprASTWantedNode::Value {
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeUnaryMinus, cur_token, prev_token);
                            *top_node_p = cur_node;
                            ast_stack.push(&mut (*cur_node).children);
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimUnaryMinus", c"NvimInvalidUnaryMinus"),
                            );
                        } else {
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeBinaryMinus, cur_token, prev_token);
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
                    kExprLexOr => {
                        add_value_if_missing!(c"E15: Unexpected or operator: %.*s".as_ptr());
                        let cur_node = new_node_with_cur_pos(kExprNodeOr, cur_token, prev_token);
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
                    kExprLexAnd => {
                        add_value_if_missing!(c"E15: Unexpected and operator: %.*s".as_ptr());
                        let cur_node = new_node_with_cur_pos(kExprNodeAnd, cur_token, prev_token);
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
                    kExprLexMultiplication => {
                        add_value_if_missing!(
                            c"E15: Unexpected multiplication-like operator: %.*s".as_ptr()
                        );
                        let cur_node = match cur_token.data.mul.r#type {
                            kExprLexMulMul => {
                                let n = new_node_with_cur_pos(
                                    kExprNodeMultiplication,
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
                            kExprLexMulDiv => {
                                let n =
                                    new_node_with_cur_pos(kExprNodeDivision, cur_token, prev_token);
                                viml_parser_highlight(
                                    pstate,
                                    cur_token.start,
                                    cur_token.len,
                                    hl_group!(is_invalid, c"NvimDivision", c"NvimInvalidDivision"),
                                );
                                n
                            }
                            _ => {
                                let n = new_node_with_cur_pos(kExprNodeMod, cur_token, prev_token);
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
                    kExprLexOption => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let cur_node =
                            new_node_with_cur_pos(kExprNodeOption, cur_token, prev_token);
                        if cur_token.r#type == kExprLexInvalid {
                            (*cur_node).data.opt.ident =
                                pline.data.add(cur_token.start.col + cur_token.len);
                            (*cur_node).data.opt.ident_len = 0;
                            (*cur_node).data.opt.scope = if cur_token.len == 3 {
                                c_int::from(*pline.data.add(cur_token.start.col + 1) as u8)
                            } else {
                                kExprOptScopeUnspecified
                            };
                        } else {
                            (*cur_node).data.opt.ident = cur_token.data.opt.name;
                            (*cur_node).data.opt.ident_len = cur_token.data.opt.len;
                            (*cur_node).data.opt.scope = cur_token.data.opt.scope;
                        }
                        *top_node_p = cur_node;
                        want_node = ExprASTWantedNode::Operator;
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            1,
                            hl_group!(is_invalid, c"NvimOptionSigil", c"NvimInvalidOptionSigil"),
                        );
                        let scope_shift = if cur_token.data.opt.scope == kExprOptScopeUnspecified {
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
                    kExprLexEnv => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let cur_node =
                            new_node_with_cur_pos(kExprNodeEnvironment, cur_token, prev_token);
                        (*cur_node).data.env.ident = pline.data.add(cur_token.start.col + 1);
                        (*cur_node).data.env.ident_len = cur_token.len - 1;
                        if (*cur_node).data.env.ident_len == 0 {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Environment variable name missing".as_ptr(),
                                cur_token.start,
                            );
                        }
                        *top_node_p = cur_node;
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
                    kExprLexNot => {
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let cur_node = new_node_with_cur_pos(kExprNodeNot, cur_token, prev_token);
                        *top_node_p = cur_node;
                        ast_stack.push(&mut (*cur_node).children);
                        viml_parser_highlight(
                            pstate,
                            cur_token.start,
                            cur_token.len,
                            hl_group!(is_invalid, c"NvimNot", c"NvimInvalidNot"),
                        );
                    }
                    kExprLexComparison => {
                        add_value_if_missing!(
                            c"E15: Expected value, got comparison operator: %.*s".as_ptr()
                        );
                        let cur_node =
                            new_node_with_cur_pos(kExprNodeComparison, cur_token, prev_token);
                        if cur_token.r#type == kExprLexInvalid {
                            (*cur_node).data.cmp.ccs = kCCStrategyUseOption;
                            (*cur_node).data.cmp.r#type = kExprCmpEqual;
                            (*cur_node).data.cmp.inv = false;
                        } else {
                            (*cur_node).data.cmp.ccs = cur_token.data.cmp.ccs;
                            (*cur_node).data.cmp.r#type = cur_token.data.cmp.r#type;
                            (*cur_node).data.cmp.inv = cur_token.data.cmp.inv;
                        }
                        is_invalid |= !viml_pexpr_handle_bop(
                            pstate,
                            &mut ast_stack,
                            cur_node,
                            &mut want_node,
                            &mut ast.err,
                        );
                        if cur_token.data.cmp.ccs != kCCStrategyUseOption {
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
                    kExprLexComma => {
                        debug_assert!(
                            !(want_node == ExprASTWantedNode::Value
                                && cur_pt == ExprASTParseType::LambdaArguments)
                        );
                        if want_node == ExprASTWantedNode::Value {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Expected value, got comma: %.*s".as_ptr(),
                                cur_token.start,
                            );
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeMissing, cur_token, prev_token);
                            (*cur_node).len = 0;
                            *top_node_p = cur_node;
                            want_node = ExprASTWantedNode::Operator;
                        }
                        if cur_pt == ExprASTParseType::LambdaArguments {
                            debug_assert!(!lambda_node.is_null());
                            (*lambda_node).r#type = kExprNodeLambda;
                            if !(*pstate).colors.is_null() {
                                (*(*pstate).colors).as_mut_slice()
                                    [(*lambda_node).data.fig.opening_hl_idx]
                                    .group =
                                    hl_group!(is_invalid, c"NvimLambda", c"NvimInvalidLambda");
                            }
                        }
                        let mut valid_comma = false;
                        if ast_stack.len() >= 2 {
                            for i in 1..ast_stack.len() {
                                let eastnode_p = ast_stack[ast_stack.len() - i - 1];
                                let eastnode_type = (**eastnode_p).r#type;
                                let eastnode_lvl = node_lvl(**eastnode_p);
                                if eastnode_type == kExprNodeLambda
                                    || eastnode_type == kExprNodeDictLiteral
                                    || eastnode_type == kExprNodeListLiteral
                                    || eastnode_type == kExprNodeCall
                                {
                                    valid_comma = true;
                                    break;
                                } else if eastnode_type == kExprNodeComma
                                    || eastnode_type == kExprNodeColon
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
                                c"E15: Comma outside of call, lambda or literal: %.*s".as_ptr(),
                                cur_token.start,
                            );
                        }
                        let cur_node = new_node_with_cur_pos(kExprNodeComma, cur_token, prev_token);
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
                    kExprLexColon => {
                        let mut is_ternary = false;
                        let mut is_subscript = false;
                        let mut valid_colon = false;
                        if ast_stack.len() >= 2 {
                            let mut can_be_ternary = true;
                            for i in 1..ast_stack.len() {
                                let eastnode_p = ast_stack[ast_stack.len() - i - 1];
                                let eastnode_type = (**eastnode_p).r#type;
                                let eastnode_lvl = node_lvl(**eastnode_p);
                                if can_be_ternary
                                    && eastnode_type == kExprNodeTernaryValue
                                    && !(**eastnode_p).data.ter.got_colon
                                {
                                    ast_stack.truncate(ast_stack.len() - i);
                                    (**eastnode_p).start = cur_token.start;
                                    (**eastnode_p).len = cur_token.len;
                                    if prev_token.r#type == kExprLexSpacing {
                                        (**eastnode_p).start = prev_token.start;
                                        (**eastnode_p).len += prev_token.len;
                                    }
                                    is_ternary = true;
                                    (**eastnode_p).data.ter.got_colon = true;
                                    add_value_if_missing!(
                                        c"E15: Expected value, got colon: %.*s".as_ptr()
                                    );
                                    ast_stack.push(&mut (*(**eastnode_p).children).next);
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == kExprNodeUnknownFigure {
                                    (**eastnode_p).r#type = kExprNodeDictLiteral;
                                    if !(*pstate).colors.is_null() {
                                        (*(*pstate).colors).as_mut_slice()
                                            [(**eastnode_p).data.fig.opening_hl_idx]
                                            .group =
                                            hl_group!(is_invalid, c"NvimDict", c"NvimInvalidDict");
                                    }
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == kExprNodeDictLiteral {
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == kExprNodeSubscript {
                                    is_subscript = true;
                                    valid_colon = true;
                                    break;
                                } else if eastnode_type == kExprNodeColon {
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
                                && (**ast_stack[ast_stack.len() - 2]).r#type == kExprNodeSubscript
                            {
                                *top_node_p =
                                    new_node_with_cur_pos(kExprNodeMissing, cur_token, prev_token);
                                (**top_node_p).len = 0;
                                want_node = ExprASTWantedNode::Operator;
                            } else {
                                add_value_if_missing!(
                                    c"E15: Expected value, got colon: %.*s".as_ptr()
                                );
                            }
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeColon, cur_token, prev_token);
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
                                    c"E15: Colon outside of dictionary or ternary operator: %.*s"
                                        .as_ptr(),
                                    cur_token.start,
                                );
                            }
                            add_value_if_missing!(c"E15: Expected value, got colon: %.*s".as_ptr());
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
                                    new_node_with_cur_pos(kExprNodeColon, cur_token, prev_token);
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
                    kExprLexBracket => {
                        if cur_token.data.brc.closing {
                            let mut new_top_node_p: *mut *mut ExprASTNode;
                            ast_stack.pop();
                            if ast_stack.is_empty() {
                                let cur_node = new_node_with_cur_pos(
                                    kExprNodeListLiteral,
                                    cur_token,
                                    prev_token,
                                );
                                (*cur_node).len = 0;
                                if want_node != ExprASTWantedNode::Value {
                                    (*cur_node).children = *top_node_p;
                                }
                                *top_node_p = cur_node;
                                new_top_node_p = top_node_p;
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected closing figure brace: %.*s".as_ptr(),
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
                                    let last_type = (***ast_stack.last().unwrap()).r#type;
                                    if last_type != kExprNodeListLiteral
                                        && last_type != kExprNodeComma
                                        && last_type != kExprNodeColon
                                    {
                                        is_invalid = true;
                                        east_set_error(
                                            pstate,
                                            &mut ast.err,
                                            c"E15: Expected value, got closing bracket: %.*s"
                                                .as_ptr(),
                                            cur_token.start,
                                        );
                                    }
                                }
                                loop {
                                    new_top_node_p = ast_stack.pop().unwrap();
                                    if ast_stack.is_empty()
                                        || (!new_top_node_p.is_null()
                                            && ((**new_top_node_p).r#type == kExprNodeListLiteral
                                                || (**new_top_node_p).r#type == kExprNodeSubscript))
                                    {
                                        break;
                                    }
                                }
                                let new_top_node = *new_top_node_p;
                                match (*new_top_node).r#type {
                                    kExprNodeListLiteral => {
                                        if pt_is_assignment(cur_pt)
                                            && (*new_top_node).children.is_null()
                                        {
                                            is_invalid = true;
                                            east_set_error(
                                                pstate,
                                                &mut ast.err,
                                                c"E475: Unable to assign to empty list: %.*s"
                                                    .as_ptr(),
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
                                    kExprNodeSubscript => {
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
                                            c"E15: Unexpected closing figure brace: %.*s".as_ptr(),
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
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeListLiteral, cur_token, prev_token);
                            *top_node_p = cur_node;
                            ast_stack.push(&mut (*cur_node).children);
                            if cur_pt == ExprASTParseType::Assignment {
                                pt_stack.push(ExprASTParseType::SingleAssignment);
                            } else if cur_pt == ExprASTParseType::SingleAssignment {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E475: Nested lists not allowed when assigning: %.*s".as_ptr(),
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
                            if prev_token.r#type == kExprLexSpacing {
                                op_missing!();
                            }
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeSubscript, cur_token, prev_token);
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
                    kExprLexFigureBrace => {
                        if cur_token.data.brc.closing {
                            let mut new_top_node_p: *mut *mut ExprASTNode;
                            ast_stack.pop();
                            if ast_stack.is_empty() {
                                let cur_node = new_node_with_cur_pos(
                                    kExprNodeUnknownFigure,
                                    cur_token,
                                    prev_token,
                                );
                                (*cur_node).data.fig.type_guesses.allow_lambda = false;
                                (*cur_node).data.fig.type_guesses.allow_dict = false;
                                (*cur_node).data.fig.type_guesses.allow_ident = false;
                                (*cur_node).len = 0;
                                if want_node != ExprASTWantedNode::Value {
                                    (*cur_node).children = *top_node_p;
                                }
                                *top_node_p = cur_node;
                                new_top_node_p = top_node_p;
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected closing figure brace: %.*s".as_ptr(),
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
                                    let last_type = (***ast_stack.last().unwrap()).r#type;
                                    if last_type != kExprNodeUnknownFigure
                                        && last_type != kExprNodeComma
                                    {
                                        is_invalid = true;
                                        east_set_error(
                                            pstate,
                                            &mut ast.err,
                                            c"E15: Expected value, got closing figure brace: %.*s"
                                                .as_ptr(),
                                            cur_token.start,
                                        );
                                    }
                                }
                                loop {
                                    new_top_node_p = ast_stack.pop().unwrap();
                                    if ast_stack.is_empty()
                                        || (!new_top_node_p.is_null()
                                            && ((**new_top_node_p).r#type
                                                == kExprNodeUnknownFigure
                                                || (**new_top_node_p).r#type
                                                    == kExprNodeDictLiteral
                                                || (**new_top_node_p).r#type
                                                    == kExprNodeCurlyBracesIdentifier
                                                || (**new_top_node_p).r#type == kExprNodeLambda))
                                    {
                                        break;
                                    }
                                }
                                let new_top_node = *new_top_node_p;
                                match (*new_top_node).r#type {
                                    kExprNodeUnknownFigure => {
                                        if (*new_top_node).children.is_null() {
                                            (*new_top_node).r#type = kExprNodeDictLiteral;
                                            let grp = hl_group!(
                                                is_invalid,
                                                c"NvimDict",
                                                c"NvimInvalidDict"
                                            );
                                            if !(*pstate).colors.is_null() {
                                                (*(*pstate).colors).as_mut_slice()
                                                    [(*new_top_node).data.fig.opening_hl_idx]
                                                    .group = grp;
                                            }
                                            viml_parser_highlight(
                                                pstate,
                                                cur_token.start,
                                                cur_token.len,
                                                grp,
                                            );
                                        } else if (*new_top_node).data.fig.type_guesses.allow_ident
                                        {
                                            (*new_top_node).r#type = kExprNodeCurlyBracesIdentifier;
                                            let grp = hl_group!(
                                                is_invalid,
                                                c"NvimCurly",
                                                c"NvimInvalidCurly"
                                            );
                                            if !(*pstate).colors.is_null() {
                                                (*(*pstate).colors).as_mut_slice()
                                                    [(*new_top_node).data.fig.opening_hl_idx]
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
                                                c"E15: Don't know what figure brace means: %.*s"
                                                    .as_ptr(),
                                                (*new_top_node).start,
                                            );
                                            let grp = hl_group!(
                                                is_invalid,
                                                c"NvimFigureBrace",
                                                c"NvimInvalidFigureBrace"
                                            );
                                            if !(*pstate).colors.is_null() {
                                                (*(*pstate).colors).as_mut_slice()
                                                    [(*new_top_node).data.fig.opening_hl_idx]
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
                                    kExprNodeDictLiteral => {
                                        viml_parser_highlight(
                                            pstate,
                                            cur_token.start,
                                            cur_token.len,
                                            hl_group!(is_invalid, c"NvimDict", c"NvimInvalidDict"),
                                        );
                                    }
                                    kExprNodeCurlyBracesIdentifier => {
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
                                    kExprNodeLambda => {
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
                                            c"E15: Unexpected closing figure brace: %.*s".as_ptr(),
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
                                let cur_node = if pt_is_assignment(cur_pt) {
                                    let n = new_node_with_cur_pos(
                                        kExprNodeCurlyBracesIdentifier,
                                        cur_token,
                                        prev_token,
                                    );
                                    (*n).data.fig.type_guesses.allow_lambda = false;
                                    (*n).data.fig.type_guesses.allow_dict = false;
                                    (*n).data.fig.type_guesses.allow_ident = true;
                                    pt_stack.push(ExprASTParseType::Expr);
                                    n
                                } else {
                                    let n = new_node_with_cur_pos(
                                        kExprNodeUnknownFigure,
                                        cur_token,
                                        prev_token,
                                    );
                                    (*n).data.fig.type_guesses.allow_lambda = true;
                                    (*n).data.fig.type_guesses.allow_dict = true;
                                    (*n).data.fig.type_guesses.allow_ident = true;
                                    n
                                };
                                if !(*pstate).colors.is_null() {
                                    (*cur_node).data.fig.opening_hl_idx =
                                        (*(*pstate).colors).len() - 1;
                                }
                                *top_node_p = cur_node;
                                ast_stack.push(&mut (*cur_node).children);
                                pt_stack.push(ExprASTParseType::LambdaArguments);
                                lambda_node = cur_node;
                            } else {
                                if prev_token.r#type == kExprLexSpacing {
                                    op_missing!();
                                }
                                let top_type = (**top_node_p).r#type;
                                if top_type == kExprNodeComplexIdentifier
                                    || top_type == kExprNodePlainIdentifier
                                    || top_type == kExprNodeCurlyBracesIdentifier
                                {
                                    let comp_node = new_node_with_cur_pos(
                                        kExprNodeComplexIdentifier,
                                        cur_token,
                                        prev_token,
                                    );
                                    (*comp_node).len = 0;
                                    (*comp_node).children = *top_node_p;
                                    *top_node_p = comp_node;
                                    ast_stack.push(&mut (*(*comp_node).children).next);
                                    let new_top_node_p = *ast_stack.last().unwrap();
                                    let cur_node = new_node_with_cur_pos(
                                        kExprNodeCurlyBracesIdentifier,
                                        cur_token,
                                        prev_token,
                                    );
                                    if !(*pstate).colors.is_null() {
                                        (*cur_node).data.fig.opening_hl_idx =
                                            (*(*pstate).colors).len();
                                    }
                                    (*cur_node).data.fig.type_guesses.allow_lambda = false;
                                    (*cur_node).data.fig.type_guesses.allow_dict = false;
                                    (*cur_node).data.fig.type_guesses.allow_ident = true;
                                    ast_stack.push(&mut (*cur_node).children);
                                    if pt_is_assignment(cur_pt) {
                                        pt_stack.push(ExprASTParseType::Expr);
                                    }
                                    want_node = ExprASTWantedNode::Value;
                                    *new_top_node_p = cur_node;
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
                    kExprLexArrow => {
                        if cur_pt == ExprASTParseType::LambdaArguments {
                            pt_stack.pop();
                            if want_node == ExprASTWantedNode::Value {
                                ast_stack.pop();
                            }
                            while (***ast_stack.last().unwrap()).r#type != kExprNodeLambda
                                && (***ast_stack.last().unwrap()).r#type != kExprNodeUnknownFigure
                            {
                                ast_stack.pop();
                            }
                            debug_assert_eq!(**ast_stack.last().unwrap(), lambda_node);
                            (*lambda_node).r#type = kExprNodeLambda;
                            if !(*pstate).colors.is_null() {
                                (*(*pstate).colors).as_mut_slice()
                                    [(*lambda_node).data.fig.opening_hl_idx]
                                    .group =
                                    hl_group!(is_invalid, c"NvimLambda", c"NvimInvalidLambda");
                            }
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeArrow, cur_token, prev_token);
                            if (*lambda_node).children.is_null() {
                                (*lambda_node).children = cur_node;
                                ast_stack.push(&mut (*lambda_node).children);
                            } else {
                                (*(*lambda_node).children).next = cur_node;
                                ast_stack.push(&mut (*(*lambda_node).children).next);
                            }
                            ast_stack.push(&mut (*cur_node).children);
                            lambda_node = std::ptr::null_mut();
                        } else {
                            is_invalid = true;
                            if want_node == ExprASTWantedNode::Value {
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Unexpected arrow: %.*s".as_ptr(),
                                    cur_token.start,
                                );
                                *top_node_p =
                                    new_node_with_cur_pos(kExprNodeMissing, cur_token, prev_token);
                                (**top_node_p).len = 0;
                                want_node = ExprASTWantedNode::Operator;
                            }
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Arrow outside of lambda: %.*s".as_ptr(),
                                cur_token.start,
                            );
                            let cur_node =
                                new_node_with_cur_pos(kExprNodeArrow, cur_token, prev_token);
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
                    kExprLexPlainIdentifier => {
                        let scope = if cur_token.r#type == kExprLexInvalid {
                            kExprVarScopeMissing
                        } else {
                            cur_token.data.var.scope
                        };
                        if want_node == ExprASTWantedNode::Value {
                            want_node = ExprASTWantedNode::Operator;
                            let cur_node = new_node_with_cur_pos(
                                if node_is_key {
                                    kExprNodePlainKey
                                } else {
                                    kExprNodePlainIdentifier
                                },
                                cur_token,
                                prev_token,
                            );
                            (*cur_node).data.var.scope = scope;
                            let scope_shift = if scope == kExprVarScopeMissing { 0 } else { 2 };
                            (*cur_node).data.var.ident =
                                pline.data.add(cur_token.start.col + scope_shift);
                            (*cur_node).data.var.ident_len = cur_token.len - scope_shift;
                            *top_node_p = cur_node;
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
                        } else if scope == kExprVarScopeMissing {
                            if prev_token.r#type == kExprLexSpacing {
                                op_missing!();
                            }
                            let top_type = (**top_node_p).r#type;
                            if top_type == kExprNodeComplexIdentifier
                                || top_type == kExprNodePlainIdentifier
                                || top_type == kExprNodeCurlyBracesIdentifier
                            {
                                let comp_node = new_node_with_cur_pos(
                                    kExprNodeComplexIdentifier,
                                    cur_token,
                                    prev_token,
                                );
                                (*comp_node).len = 0;
                                (*comp_node).children = *top_node_p;
                                *top_node_p = comp_node;
                                ast_stack.push(&mut (*(*comp_node).children).next);
                                let new_top_node_p = *ast_stack.last().unwrap();
                                let cur_node = new_node_with_cur_pos(
                                    kExprNodePlainIdentifier,
                                    cur_token,
                                    prev_token,
                                );
                                (*cur_node).data.var.scope = scope;
                                (*cur_node).data.var.ident = pline.data.add(cur_token.start.col);
                                (*cur_node).data.var.ident_len = cur_token.len;
                                want_node = ExprASTWantedNode::Operator;
                                *new_top_node_p = cur_node;
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
                    kExprLexNumber => {
                        if want_node != ExprASTWantedNode::Value {
                            op_missing!();
                        }
                        let cur_node = if node_is_key {
                            let n = new_node_with_cur_pos(kExprNodePlainKey, cur_token, prev_token);
                            (*n).data.var.ident = pline.data.add(cur_token.start.col);
                            (*n).data.var.ident_len = cur_token.len;
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
                        } else if cur_token.data.num.is_float {
                            let n = new_node_with_cur_pos(kExprNodeFloat, cur_token, prev_token);
                            (*n).data.flt.value = cur_token.data.num.val.floating;
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimFloat", c"NvimInvalidFloat"),
                            );
                            n
                        } else {
                            let n = new_node_with_cur_pos(kExprNodeInteger, cur_token, prev_token);
                            (*n).data.num.value = cur_token.data.num.val.integer;
                            let prefix_length =
                                BASE_TO_PREFIX_LENGTH[cur_token.data.num.base as usize];
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
                        *top_node_p = cur_node;
                    }
                    kExprLexDot => {
                        add_value_if_missing!(c"E15: Unexpected dot: %.*s".as_ptr());
                        let cur_node = if prev_token.r#type == kExprLexSpacing {
                            if cur_pt == ExprASTParseType::Assignment {
                                is_invalid = true;
                                east_set_error(
                                    pstate,
                                    &mut ast.err,
                                    c"E15: Cannot concatenate in assignments: %.*s".as_ptr(),
                                    cur_token.start,
                                );
                            }
                            let n = new_node_with_cur_pos(kExprNodeConcat, cur_token, prev_token);
                            viml_parser_highlight(
                                pstate,
                                cur_token.start,
                                cur_token.len,
                                hl_group!(is_invalid, c"NvimConcat", c"NvimInvalidConcat"),
                            );
                            n
                        } else {
                            let n = new_node_with_cur_pos(
                                kExprNodeConcatOrSubscript,
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
                    kExprLexParenthesis => {
                        if cur_token.data.brc.closing {
                            let mut no_paren_closing_error = false;
                            if want_node == ExprASTWantedNode::Value {
                                if ast_stack.len() > 1 {
                                    let prev_top_node = *ast_stack[ast_stack.len() - 2];
                                    if (*prev_top_node).r#type == kExprNodeCall {
                                        ast_stack.pop();
                                        no_paren_closing_error = true;
                                    }
                                }
                                if !no_paren_closing_error {
                                    is_invalid = true;
                                    east_set_error(
                                        pstate,
                                        &mut ast.err,
                                        c"E15: Expected value, got parenthesis: %.*s".as_ptr(),
                                        cur_token.start,
                                    );
                                    let cur_node = new_node_with_cur_pos(
                                        kExprNodeMissing,
                                        cur_token,
                                        prev_token,
                                    );
                                    (*cur_node).len = 0;
                                    *top_node_p = cur_node;
                                }
                            } else {
                                ast_stack.pop();
                            }
                            let mut new_top_node_p: *mut *mut ExprASTNode = std::ptr::null_mut();
                            while !ast_stack.is_empty()
                                && (new_top_node_p.is_null()
                                    || ((*(*new_top_node_p)).r#type != kExprNodeNested
                                        && (*(*new_top_node_p)).r#type != kExprNodeCall))
                            {
                                new_top_node_p = ast_stack.pop().unwrap();
                            }
                            if !new_top_node_p.is_null()
                                && ((*(*new_top_node_p)).r#type == kExprNodeNested
                                    || (*(*new_top_node_p)).r#type == kExprNodeCall)
                            {
                                if (*(*new_top_node_p)).r#type == kExprNodeNested {
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
                                    c"E15: Unexpected closing parenthesis: %.*s".as_ptr(),
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
                                let cur_node = viml_pexpr_new_node(kExprNodeNested);
                                (*cur_node).start = cur_token.start;
                                (*cur_node).len = 0;
                                (*cur_node).children = *new_top_node_p;
                                *new_top_node_p = cur_node;
                            }
                            ast_stack.push(new_top_node_p);
                            want_node = ExprASTWantedNode::Operator;
                        } else {
                            match want_node {
                                ExprASTWantedNode::Value => {
                                    let cur_node = new_node_with_cur_pos(
                                        kExprNodeNested,
                                        cur_token,
                                        prev_token,
                                    );
                                    *top_node_p = cur_node;
                                    ast_stack.push(&mut (*cur_node).children);
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
                                    if prev_token.r#type == kExprLexSpacing {
                                        let top_type = (**top_node_p).r#type;
                                        if top_type != kExprNodePlainIdentifier
                                            && top_type != kExprNodeComplexIdentifier
                                            && top_type != kExprNodeCurlyBracesIdentifier
                                        {
                                            op_missing!();
                                        }
                                    }
                                    let cur_node =
                                        new_node_with_cur_pos(kExprNodeCall, cur_token, prev_token);
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
                    kExprLexQuestion => {
                        add_value_if_missing!(
                            c"E15: Expected value, got question mark: %.*s".as_ptr()
                        );
                        let cur_node =
                            new_node_with_cur_pos(kExprNodeTernary, cur_token, prev_token);
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
                        let ter_val_node =
                            new_node_with_cur_pos(kExprNodeTernaryValue, cur_token, prev_token);
                        (*ter_val_node).data.ter.got_colon = false;
                        **ast_stack.last().unwrap() = ter_val_node;
                        ast_stack.push(&mut (*ter_val_node).children);
                    }
                    kExprLexDoubleQuotedString | kExprLexSingleQuotedString => {
                        let is_double = tok_type == kExprLexDoubleQuotedString;
                        if !cur_token.data.str.closed {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                if is_double {
                                    c"E114: Missing double quote: %.*s".as_ptr()
                                } else {
                                    c"E115: Missing single quote: %.*s".as_ptr()
                                },
                                cur_token.start,
                            );
                        }
                        if want_node == ExprASTWantedNode::Operator {
                            op_missing!();
                        }
                        let cur_node = new_node_with_cur_pos(
                            if is_double {
                                kExprNodeDoubleQuotedString
                            } else {
                                kExprNodeSingleQuotedString
                            },
                            cur_token,
                            prev_token,
                        );
                        *top_node_p = cur_node;
                        parse_quoted_string(pstate, cur_node, cur_token, &ast_stack, is_invalid);
                        want_node = ExprASTWantedNode::Operator;
                    }
                    kExprLexAssignment => {
                        if cur_pt == ExprASTParseType::Assignment {
                            pt_stack.pop();
                        } else if cur_pt == ExprASTParseType::SingleAssignment {
                            pt_stack.truncate(pt_stack.len() - 2);
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E475: Expected closing bracket to end list assignment lvalue: %.*s"
                                    .as_ptr(),
                                cur_token.start,
                            );
                        } else {
                            is_invalid = true;
                            east_set_error(
                                pstate,
                                &mut ast.err,
                                c"E15: Misplaced assignment: %.*s".as_ptr(),
                                cur_token.start,
                            );
                        }
                        add_value_if_missing!(c"E15: Unexpected assignment: %.*s".as_ptr());
                        let cur_node =
                            new_node_with_cur_pos(kExprNodeAssignment, cur_token, prev_token);
                        (*cur_node).data.ass.r#type = cur_token.data.ass.r#type;
                        let hl_grp = match cur_token.data.ass.r#type {
                            kExprAsgnPlain => hl_group!(
                                is_invalid,
                                c"NvimPlainAssignment",
                                c"NvimInvalidPlainAssignment"
                            ),
                            kExprAsgnAdd => hl_group!(
                                is_invalid,
                                c"NvimAssignmentWithAddition",
                                c"NvimInvalidAssignmentWithAddition"
                            ),
                            kExprAsgnSubtract => hl_group!(
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
                c"E15: Expected value, got EOC: %.*s".as_ptr(),
                (*pstate).pos,
            );
        } else if ast_stack.len() != 1 {
            ast_stack.pop();
            while ast.err.msg.is_null()
                && let Some(node_p) = ast_stack.pop()
            {
                let cur_node = *node_p;
                debug_assert!(!cur_node.is_null());
                match (*cur_node).r#type {
                    kExprNodeOpMissing | kExprNodeMissing => {}
                    kExprNodeCall => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E116: Missing closing parenthesis for function call: %.*s".as_ptr(),
                            (*cur_node).start,
                        );
                    }
                    kExprNodeNested => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E110: Missing closing parenthesis for nested expression: %.*s"
                                .as_ptr(),
                            (*cur_node).start,
                        );
                    }
                    kExprNodeListLiteral => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E697: Missing end of List ']': %.*s".as_ptr(),
                            (*cur_node).start,
                        );
                    }
                    kExprNodeDictLiteral => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E723: Missing end of Dictionary '}': %.*s".as_ptr(),
                            (*cur_node).start,
                        );
                    }
                    kExprNodeUnknownFigure => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E15: Missing closing figure brace: %.*s".as_ptr(),
                            (*cur_node).start,
                        );
                    }
                    kExprNodeLambda => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E15: Missing closing figure brace for lambda: %.*s".as_ptr(),
                            (*cur_node).start,
                        );
                    }
                    kExprNodeCurlyBracesIdentifier
                    | kExprNodeInteger
                    | kExprNodeFloat
                    | kExprNodeSingleQuotedString
                    | kExprNodeDoubleQuotedString
                    | kExprNodeOption
                    | kExprNodeEnvironment
                    | kExprNodeRegister
                    | kExprNodePlainIdentifier
                    | kExprNodePlainKey => {
                        std::process::abort();
                    }
                    kExprNodeTernaryValue if !(*cur_node).data.ter.got_colon => {
                        east_set_error(
                            pstate,
                            &mut ast.err,
                            c"E109: Missing ':' after '?': %.*s".as_ptr(),
                            (*cur_node).start,
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
    use crate::viml::parser::parser::{
        ParserHighlight, parser_simple_get_line, viml_parser_destroy,
    };
    use std::ffi::CStr;

    unsafe fn with_pstate<R>(input: &[u8], f: impl FnOnce(&mut ParserState) -> R) -> R {
        let lines = [
            ParserLine {
                data: input.as_ptr().cast::<c_char>(),
                size: input.len(),
                allocated: false,
            },
            ParserLine {
                data: std::ptr::null(),
                size: 0,
                allocated: false,
            },
        ];
        let mut cursor = lines.as_ptr();
        let mut colors = ParserHighlight::new();
        let mut pstate = ParserState {
            reader: crate::viml::parser::parser::ParserInputReader {
                get_line: Some(parser_simple_get_line),
                cookie: &mut cursor,
                lines: Vec::new(),
                conv: crate::mbyte::vimconv_T {
                    vc_type: crate::viml::parser::parser::CONV_NONE,
                    vc_factor: 1,
                    vc_fd: std::ptr::null_mut(),
                    vc_fail: false,
                },
            },
            pos: ParserPosition { line: 0, col: 0 },
            colors: &mut colors,
        };
        unsafe {
            let res = f(&mut pstate);
            drop(colors);
            viml_parser_destroy(&mut pstate);
            res
        }
    }

    unsafe fn lex_one(input: &[u8], flags: c_int) -> LexExprToken {
        unsafe { with_pstate(input, |pstate| viml_pexpr_next_token(pstate, flags)) }
    }

    #[test]
    fn lexer_tokens_and_flags() {
        unsafe {
            let t = lex_one(b"(", 0);
            assert_eq!(t.r#type, kExprLexParenthesis);
            assert!(!t.data.brc.closing);

            let t = lex_one(b")]", 0);
            assert_eq!(t.r#type, kExprLexParenthesis);
            assert!(t.data.brc.closing);
            assert_eq!(t.len, 1);

            let t = lex_one(b"0x123", 0);
            assert_eq!(t.r#type, kExprLexNumber);
            assert!(!t.data.num.is_float);
            assert_eq!(t.data.num.base, 16);
            assert_eq!(t.data.num.val.integer, 0x123);

            let t = lex_one(b"0b100111", 0);
            assert_eq!(t.r#type, kExprLexNumber);
            assert_eq!(t.data.num.base, 2);
            assert_eq!(t.data.num.val.integer, 39);

            let t = lex_one(b"0123", 0);
            assert_eq!(t.r#type, kExprLexNumber);
            assert_eq!(t.data.num.base, 8);
            assert_eq!(t.data.num.val.integer, 83);

            // Without kELFlagAllowFloat, "2.5" stops at "2".
            let t = lex_one(b"2.5", 0);
            assert_eq!(t.r#type, kExprLexNumber);
            assert!(!t.data.num.is_float);
            assert_eq!(t.len, 1);
            assert_eq!(t.data.num.val.integer, 2);

            // With kELFlagAllowFloat, "2.5e-2" parses as float.
            let t = lex_one(b"2.5e-2", kELFlagAllowFloat);
            assert_eq!(t.r#type, kExprLexNumber);
            assert!(t.data.num.is_float);
            assert_eq!(t.len, 6);
            assert!((t.data.num.val.floating - 0.025).abs() < 1e-12);

            // Comparisons and case strategies
            let t = lex_one(b"==#", 0);
            assert_eq!(t.r#type, kExprLexComparison);
            assert_eq!(t.data.cmp.r#type, kExprCmpEqual);
            assert!(!t.data.cmp.inv);
            assert_eq!(t.data.cmp.ccs, kCCStrategyMatchCase);

            let t = lex_one(b"!~?", 0);
            assert_eq!(t.r#type, kExprLexComparison);
            assert_eq!(t.data.cmp.r#type, kExprCmpMatches);
            assert!(t.data.cmp.inv);
            assert_eq!(t.data.cmp.ccs, kCCStrategyIgnoreCase);

            // `is` / `isnot` with and without kELFlagIsNotCmp
            let t = lex_one(b"isnot#", 0);
            assert_eq!(t.r#type, kExprLexComparison);
            assert_eq!(t.data.cmp.r#type, kExprCmpIdentical);
            assert!(t.data.cmp.inv);
            assert_eq!(t.data.cmp.ccs, kCCStrategyMatchCase);

            let t = lex_one(b"isnot#foo", kELFlagIsNotCmp);
            assert_eq!(t.r#type, kExprLexPlainIdentifier);
            assert_eq!(t.len, 9);
            assert!(t.data.var.autoload);

            // Scope and kELFlagForbidScope
            let t = lex_one(b"g:test#var", 0);
            assert_eq!(t.r#type, kExprLexPlainIdentifier);
            assert_eq!(t.data.var.scope, b'g' as c_int);
            assert!(t.data.var.autoload);

            let t = lex_one(b"g:test", kELFlagForbidScope);
            assert_eq!(t.r#type, kExprLexPlainIdentifier);
            assert_eq!(t.len, 1);
            assert_eq!(t.data.var.scope, 0);

            // Options, registers, env, strings, assignments
            let t = lex_one(b"&l:opt", 0);
            assert_eq!(t.r#type, kExprLexOption);
            assert_eq!(t.data.opt.scope, b'l' as c_int);
            assert_eq!(t.data.opt.len, 3);

            let t = lex_one(b"&g:", 0);
            assert_eq!(t.r#type, kExprLexInvalid);
            assert_eq!(
                &CStr::from_ptr(t.data.err.msg).to_bytes()[4..],
                b": Option name missing: %.*s"
            );

            let t = lex_one(b"@a", 0);
            assert_eq!(t.r#type, kExprLexRegister);
            assert_eq!(t.data.reg.name, b'a' as c_int);

            let t = lex_one(b"'x'''", 0);
            assert_eq!(t.r#type, kExprLexSingleQuotedString);
            assert!(t.data.str.closed);

            let t = lex_one(b"\"x\\\"", 0);
            assert_eq!(t.r#type, kExprLexDoubleQuotedString);
            assert!(!t.data.str.closed);

            let t = lex_one(b".=", 0);
            assert_eq!(t.r#type, kExprLexAssignment);
            assert_eq!(t.data.ass.r#type, kExprAsgnConcat);

            // Peek does not advance pstate.pos
            with_pstate(b"abc", |pstate| {
                let t = viml_pexpr_next_token(pstate, kELFlagPeek);
                assert_eq!(t.r#type, kExprLexPlainIdentifier);
                assert_eq!(t.len, 3);
                assert_eq!(pstate.pos.col, 0);
            });

            // EOC and kELFlagForbidEOC
            let t = lex_one(b"|", 0);
            assert_eq!(t.r#type, kExprLexEOC);
            let t = lex_one(b"|", kELFlagForbidEOC);
            assert_eq!(t.r#type, kExprLexInvalid);
        }
    }

    #[test]
    fn lex_and_parse_expressions_and_errors() {
        unsafe {
            // Binary precedence: 1 + 2 * 3
            with_pstate(b"1 + 2 * 3", |pstate| {
                let ast = viml_pexpr_parse(pstate, 0);
                assert!(ast.err.msg.is_null());
                assert!(!ast.root.is_null());
                assert_eq!((*ast.root).r#type, kExprNodeBinaryPlus);
                let left = (*ast.root).children;
                let right = (*left).next;
                assert_eq!((*left).r#type, kExprNodeInteger);
                assert_eq!((*right).r#type, kExprNodeMultiplication);
                assert!(!(*pstate.colors).is_empty());
                viml_pexpr_free_ast(ast);
            });

            // Ternary, lambda, dictionary, list, call, subscript
            with_pstate(b"cond ? {x -> [x, {'k': f(x)[0]}]} : 0", |pstate| {
                let ast = viml_pexpr_parse(pstate, 0);
                assert!(ast.err.msg.is_null());
                assert_eq!((*ast.root).r#type, kExprNodeTernary);
                viml_pexpr_free_ast(ast);
            });

            // Assignment with kExprFlagsParseLet
            with_pstate(b"[a, b] = [1, 2]", |pstate| {
                let ast = viml_pexpr_parse(pstate, kExprFlagsParseLet);
                assert!(ast.err.msg.is_null());
                assert_eq!((*ast.root).r#type, kExprNodeAssignment);
                assert_eq!((*ast.root).data.ass.r#type, kExprAsgnPlain);
                viml_pexpr_free_ast(ast);
            });

            with_pstate(b"a += 1", |pstate| {
                let ast = viml_pexpr_parse(pstate, kExprFlagsParseLet);
                assert!(ast.err.msg.is_null());
                assert_eq!((*ast.root).r#type, kExprNodeAssignment);
                assert_eq!((*ast.root).data.ass.r#type, kExprAsgnAdd);
                viml_pexpr_free_ast(ast);
            });

            // Syntax errors report expected diagnostics
            with_pstate(b"1 + (2 * 3", |pstate| {
                let ast = viml_pexpr_parse(pstate, 0);
                assert!(!ast.err.msg.is_null());
                assert_eq!(
                    &CStr::from_ptr(ast.err.msg).to_bytes()[4..],
                    b": Missing closing parenthesis for nested expression: %.*s"
                );
                viml_pexpr_free_ast(ast);
            });

            with_pstate(b"cond ? 1", |pstate| {
                let ast = viml_pexpr_parse(pstate, 0);
                assert!(!ast.err.msg.is_null());
                assert_eq!(
                    &CStr::from_ptr(ast.err.msg).to_bytes()[4..],
                    b": Missing ':' after '?': %.*s"
                );
                viml_pexpr_free_ast(ast);
            });
        }
    }
}
