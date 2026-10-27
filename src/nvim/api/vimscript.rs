//! Vimscript execution, evaluation, and expression parsing API (`src/nvim/api/vimscript.c`).

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::helpers::{
    TryState, add_c, api_set_error, api_set_sctx, arena_array, arena_dict, arena_string, array_obj,
    boolean_obj, cstr_as_string, dict_obj, error_set, float_obj, integer_obj, nil_obj, put_c,
    string_obj, try_enter, try_leave,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType, api_err_exp,
};
use crate::eval::typval::{VarType, dict_T, tv_clear, tv_dict_find, typval_T};
use crate::eval::userfunc::{call_func, funcexe_T};
use crate::eval::{clear_evalarg, eval0};
use crate::ex_docmd::do_cmdline_cmd;
use crate::globals::*;
use crate::lua::converter::Boolean;
use crate::memory::Arena;
use crate::msgpack_rpc::packer::Integer;
use crate::runtime::do_source_str;
use crate::viml::parser::expressions::{
    ExprASTNode, ccs_tab, east_node_type_tab, eltkn_cmp_type_tab, expr_asgn_type_tab,
    kExprAsgnPlain, kExprFlagsDisallowEOC, kExprFlagsMulti, kExprFlagsParseLet,
    kExprNodeAssignment, kExprNodeComparison, kExprNodeDoubleQuotedString, kExprNodeEnvironment,
    kExprNodeFloat, kExprNodeInteger, kExprNodeOption, kExprNodePlainIdentifier, kExprNodePlainKey,
    kExprNodeRegister, kExprNodeSingleQuotedString, viml_pexpr_parse,
};
use crate::viml::parser::parser::{
    ParserHighlight, ParserInputReader, ParserLine, ParserPosition, ParserState,
};
use std::ffi::{CStr, c_char, c_int, c_uint};

const API_INTEGER_MAX: u64 = i64::MAX as u64;

#[repr(C)]
#[derive(Default)]
pub struct KeyDict_exec_opts {
    pub output: Boolean,
}

use crate::eval::funcs::MAX_FUNC_ARGS;
use crate::math::{FAIL, OK};

static mut EVAL_RECURSIVE: c_int = 0;
static mut CALL_FUNC_RECURSIVE: c_int = 0;

use crate::api::private::helpers::cstr_to_string;

#[inline]
unsafe fn cbuf_to_arena_obj(arena: *mut Arena, s: *const c_char, len: usize) -> Object {
    unsafe {
        string_obj(arena_string(
            arena,
            NvimString {
                data: s.cast_mut(),
                size: len,
            },
        ))
    }
}

/// Executes Vimscript (multiline block of Ex commands), like anonymous
/// |:source|.
///
/// Unlike |nvim_command()| this function supports heredocs, script-scope (s:),
/// etc.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @see |execute()|
/// @see |nvim_command()|
/// @see |nvim_cmd()|
///
/// @param src      Vimscript code
/// @param opts  Optional parameters.
///           - output: (boolean, default false) Whether to capture and return
///                     all (non-error, non-shell |:!|) output.
/// @param[out] err Error details (Vim error), if any
/// @return Dict containing information about execution, with these keys:
///       - output: (string|nil) Output if `opts.output` is true.
// DLLEXPORT Dict nvim_exec2(uint64_t channel_id, String src, Dict(exec_opts) *opts,
//                           Error *err) FUNC_API_SINCE(11) FUNC_API_RET_ALLOC;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_exec2(
    channel_id: u64,
    src: NvimString,
    opts: *mut KeyDict_exec_opts,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut result = Dict::new();
        let output = exec_impl(channel_id, src, opts, err);
        if error_set(err) {
            return result;
        }
        if (*opts).output {
            result.push(KeyValuePair {
                key: cstr_to_string(c"output".as_ptr()),
                value: string_obj(output),
            });
        }
        result
    }
}

// DLLEXPORT String exec_impl(uint64_t channel_id, String src, Dict(exec_opts) *opts, Error *err);
pub unsafe fn exec_impl(
    channel_id: u64,
    src: NvimString,
    opts: *mut KeyDict_exec_opts,
    err: *mut Error,
) -> NvimString {
    unsafe {
        let save_msg_silent = msg_silent;
        let save_redir_off = redir_off;
        let save_capture_ga = capture_ga;
        let save_msg_col = msg_col;
        let mut capture_local: Vec<c_char> = Vec::new();
        if (*opts).output {
            capture_ga = &raw mut capture_local;
        }

        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        if (*opts).output {
            msg_silent += 1;
            redir_off = false;
            msg_col = 0;
        }

        let save_current_sctx = api_set_sctx(channel_id);
        do_source_str(src.data, c"nvim_exec2()".as_ptr());
        if (*opts).output {
            capture_ga = save_capture_ga;
            msg_silent = save_msg_silent;
            redir_off = save_redir_off;
            msg_col = save_msg_col;
        }
        current_sctx = save_current_sctx;
        try_leave(&tstate, err);

        if !error_set(err) && (*opts).output && capture_local.len() > 1 {
            let slice = if capture_local[0] == b'\n' as c_char {
                &capture_local[1..]
            } else {
                &capture_local[..]
            };
            return crate::api::private::helpers::cbuf_to_string(slice.as_ptr(), slice.len());
        }

        NvimString {
            data: std::ptr::null_mut(),
            size: 0,
        }
    }
}

/// Executes an Ex command.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// Prefer |nvim_cmd()| or |nvim_exec2()| instead. To modify an Ex command in a structured way
/// before executing it, modify the result of |nvim_parse_cmd()| then pass it to |nvim_cmd()|.
///
/// @param cmd  Ex command string
/// @param[out] err Error details (Vim error), if any
// DLLEXPORT void nvim_command(String cmd, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_command(cmd: NvimString, err: *mut Error) {
    unsafe {
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        do_cmdline_cmd(cmd.data);
        try_leave(&tstate, err);
    }
}

/// Evaluates a Vimscript |expression|. Dicts and Lists are recursively expanded.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @param expr     Vimscript expression string
/// @param[out] err Error details, if any
/// @return         Evaluation result or expanded object
// DLLEXPORT Object nvim_eval(String expr, Arena *arena, Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_eval(expr: NvimString, arena: *mut Arena, err: *mut Error) -> Object {
    unsafe {
        let mut rv = nil_obj();

        if EVAL_RECURSIVE == 0 {
            force_abort = false;
            suppress_errthrow = false;
            did_throw = false;
            did_emsg = 0;
        }
        EVAL_RECURSIVE += 1;

        let mut rettv: typval_T = typval_T::new();
        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let ok = eval0(
            expr.data,
            &mut rettv,
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(EVALARG_EVALUATE),
        );
        clear_evalarg(
            std::ptr::addr_of_mut!(EVALARG_EVALUATE),
            std::ptr::null_mut(),
        );
        try_leave(&tstate, err);

        if !error_set(err) {
            if ok == FAIL {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeException,
                    c"Failed to evaluate expression: '%.*s'".as_ptr(),
                    256 as c_int,
                    expr.data,
                );
            } else {
                rv = vim_to_object(&mut rettv, arena, false);
            }
        }

        tv_clear(&mut rettv);
        EVAL_RECURSIVE -= 1;

        rv
    }
}

unsafe fn call_function_impl(
    channel_id: u64,
    fn_name: NvimString,
    args: Array,
    self_dict: *mut dict_T,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut rv = nil_obj();

        if args.size > MAX_FUNC_ARGS {
            api_set_error!(
                err,
                ErrorType::kErrorTypeValidation,
                c"Function called with too many arguments".as_ptr(),
            );
            return rv;
        }

        let mut vim_args: [typval_T; MAX_FUNC_ARGS + 1] = [typval_T::new(); MAX_FUNC_ARGS + 1];
        let mut i = 0usize;
        while i < args.size {
            object_to_vim(args[i], &mut vim_args[i], err);
            i += 1;
        }

        if CALL_FUNC_RECURSIVE == 0 {
            force_abort = false;
            suppress_errthrow = false;
            did_throw = false;
            did_emsg = 0;
        }
        CALL_FUNC_RECURSIVE += 1;

        let mut rettv: typval_T = typval_T::new();
        let lnum = (*curwin).w_cursor.lnum;
        let mut funcexe = funcexe_T {
            fe_argv_func: None,
            fe_firstline: lnum,
            fe_lastline: lnum,
            fe_doesrange: false,
            fe_evaluate: true,
            fe_partial: std::ptr::null_mut(),
            fe_selfdict: self_dict,
            fe_basetv: std::ptr::null_mut(),
            fe_found_var: false,
        };

        let mut tstate = TryState::new();
        try_enter(&mut tstate);
        let save_current_sctx = api_set_sctx(channel_id);
        let _ = call_func(
            fn_name.data,
            fn_name.size as c_int,
            &mut rettv,
            args.size as c_int,
            vim_args.as_mut_ptr(),
            &mut funcexe,
        );
        current_sctx = save_current_sctx;
        try_leave(&tstate, err);

        if !error_set(err) {
            rv = vim_to_object(&mut rettv, arena, false);
        }

        tv_clear(&mut rettv);
        CALL_FUNC_RECURSIVE -= 1;

        while i > 0 {
            i -= 1;
            tv_clear(&mut vim_args[i]);
        }

        rv
    }
}

/// Calls a Vimscript function with the given arguments.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @param fn       Function to call
/// @param args     Function arguments packed in an Array
/// @param[out] err Error details, if any
/// @return Result of the function call
// DLLEXPORT Object nvim_call_function(uint64_t channel_id, String fn, Array args, Arena *arena,
//                                     Error *err) FUNC_API_SINCE(1);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_call_function(
    channel_id: u64,
    fn_name: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe { call_function_impl(channel_id, fn_name, args, std::ptr::null_mut(), arena, err) }
}

/// Calls a Vimscript |Dictionary-function| with the given arguments.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @param dict Dict, or String evaluating to a Vimscript |self| dict
/// @param fn Name of the function defined on the Vimscript dict
/// @param args Function arguments packed in an Array
/// @param[out] err Error details, if any
/// @return Result of the function call
// DLLEXPORT Object nvim_call_dict_function(uint64_t channel_id, Object dict, String fn, Array args,
//                                          Arena *arena, Error *err) FUNC_API_SINCE(4);
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_call_dict_function(
    channel_id: u64,
    dict: Object,
    mut fn_name: NvimString,
    args: Array,
    arena: *mut Arena,
    err: *mut Error,
) -> Object {
    unsafe {
        let mut rv = nil_obj();
        let mut rettv: typval_T = typval_T::new();
        let mut mustfree = false;

        match dict.obj_type {
            ObjectType::kObjectTypeString => {
                let mut tstate = TryState::new();
                try_enter(&mut tstate);
                let eval_ret = eval0(
                    dict.data.string().data,
                    &mut rettv,
                    std::ptr::null_mut(),
                    std::ptr::addr_of_mut!(EVALARG_EVALUATE),
                );
                clear_evalarg(
                    std::ptr::addr_of_mut!(EVALARG_EVALUATE),
                    std::ptr::null_mut(),
                );
                try_leave(&tstate, err);
                if error_set(err) {
                    return rv;
                }
                if eval_ret != OK {
                    std::process::abort();
                }
                mustfree = true;
            }
            ObjectType::kObjectTypeDict => {
                object_to_vim(dict, &mut rettv, err);
            }
            _ => {
                api_err_exp(
                    err,
                    c"dict argument".as_ptr(),
                    c"String or Dict".as_ptr(),
                    std::ptr::null(),
                );
                return rv;
            }
        }

        'body: {
            let self_dict = rettv.vval.v_dict();
            if rettv.v_type != VarType::VAR_DICT || self_dict.is_null() {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"dict not found".as_ptr(),
                );
                break 'body;
            }

            if !fn_name.data.is_null()
                && fn_name.size > 0
                && dict.obj_type != ObjectType::kObjectTypeDict
            {
                let di = tv_dict_find(self_dict, fn_name.data, fn_name.size as isize);
                if di.is_null() {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Not found: %s".as_ptr(),
                        fn_name.data,
                    );
                    break 'body;
                }
                if (*di).di_tv.v_type == VarType::VAR_PARTIAL {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"partial function not supported".as_ptr(),
                    );
                    break 'body;
                }
                if (*di).di_tv.v_type != VarType::VAR_FUNC {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Not a function: %s".as_ptr(),
                        fn_name.data,
                    );
                    break 'body;
                }
                let s = (*di).di_tv.vval.v_string();
                fn_name = NvimString {
                    data: s,
                    size: CStr::from_ptr(s).to_bytes().len(),
                };
            }

            if fn_name.data.is_null() || fn_name.size == 0 {
                api_set_error!(
                    err,
                    ErrorType::kErrorTypeValidation,
                    c"Invalid function name: %s".as_ptr(),
                    c"(empty)".as_ptr(),
                );
                break 'body;
            }

            rv = call_function_impl(channel_id, fn_name, args, self_dict, arena, err);
        }

        if mustfree {
            tv_clear(&mut rettv);
        }

        rv
    }
}

/// Parse a Vimscript expression.
///
/// @param[in]  expr  Expression to parse. Always treated as a single line.
/// @param[in]  flags Flags:
///                    - "E" if EOC tokens are not allowed (determines whether
///                      they will stop parsing process or be recognized as an
///                      operator/space, though also yielding an error).
///                    - "l" when needing to start parsing with lvalues for
///                      ":let" or ":for".
///                    - "m" if multiple expressions in a row are allowed (only
///                      the first one will be parsed),
///                    - Common flag sets:
///                      - "E" to parse like for `"<C-r>="`.
///                      - "lm" to parse for ":let".
///                      - "m" to parse like for `":echo"`.
///                      - empty string for ":call".
/// @param[in]  hl  If true, return value will also include "highlight"
///                        key containing array of 4-tuples (arrays) (Integer,
///                        Integer, Integer, String), where first three numbers
///                        define the highlighted region and represent line,
///                        starting column and ending column (latter exclusive:
///                        one should highlight region [start_col, end_col)).
///
/// @return
///      - AST: top-level dict with these keys:
///        - "error": Dict with error, present only if parser saw some
///                 error. Contains the following keys:
///          - "message": String, error message in printf format, translated.
///                       Must contain exactly one `%.*s`.
///          - "arg": String, error message argument.
///        - "len": Amount of bytes successfully parsed. With flags equal to ""
///                 that should be equal to the length of expr string.
///                 ("Successfully parsed" here means "participated in AST
///                  creation", not "till the first error".)
///        - "ast": AST, either nil or a dict with these keys:
///          - "type": node type, one of the value names from ExprASTNodeType
///                    stringified without "kExprNode" prefix.
///          - "start": a pair `[line, column]` describing where node is "started"
///                     where "line" is always 0 (will not be 0 if you will be
///                     using this API on e.g. ":let", but that is not
///                     present yet). Both elements are Integers.
///          - "len": “length” of the node. This and "start" are there for
///                   debugging purposes primary (debugging parser and providing
///                   debug information).
///          - "children": a list of nodes described in top/"ast". There always
///                        is zero, one or two children, key will not be present
///                        if node has no children. Maximum number of children
///                        may be found in node_maxchildren array.
///      - Local values (present only for certain nodes):
///        - "scope": a single Integer, specifies scope for "Option" and
///                   "PlainIdentifier" nodes. For "Option" it is one of
///                   ExprOptScope values, for "PlainIdentifier" it is one of
///                   ExprVarScope values.
///        - "ident": identifier (without scope, if any), present for "Option",
///                   "PlainIdentifier", "PlainKey" and "Environment" nodes.
///        - "name": Integer, register name (one character) or -1. Only present
///                for "Register" nodes.
///        - "cmp_type": String, comparison type, one of the value names from
///                      ExprComparisonType, stringified without "kExprCmp"
///                      prefix. Only present for "Comparison" nodes.
///        - "ccs_strategy": String, case comparison strategy, one of the
///                          value names from ExprCaseCompareStrategy,
///                          stringified without "kCCStrategy" prefix. Only
///                          present for "Comparison" nodes.
///        - "augmentation": String, augmentation type for "Assignment" nodes.
///                          Is either an empty string, "Add", "Subtract" or
///                          "Concat" for "=", "+=", "-=" or ".=" respectively.
///        - "invert": Boolean, true if result of comparison needs to be
///                    inverted. Only present for "Comparison" nodes.
///        - "ivalue": Integer, integer value for "Integer" nodes.
///        - "fvalue": Float, floating-point value for "Float" nodes.
///        - "svalue": String, value for "SingleQuotedString" and
///                    "DoubleQuotedString" nodes.
/// @param[out] err Error details, if any
// DLLEXPORT Dict nvim_parse_expression(String expr, String flags, Boolean hl, Arena *arena,
//                                      Error *err) FUNC_API_SINCE(4) FUNC_API_FAST;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nvim_parse_expression(
    expr: NvimString,
    flags: NvimString,
    hl: Boolean,
    arena: *mut Arena,
    err: *mut Error,
) -> Dict {
    unsafe {
        let mut pflags: c_int = 0;
        for i in 0..flags.size {
            let b = *flags.data.add(i) as u8;
            match b {
                b'm' => pflags |= kExprFlagsMulti,
                b'E' => pflags |= kExprFlagsDisallowEOC,
                b'l' => pflags |= kExprFlagsParseLet,
                0 => {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Invalid flag: '\\0' (%u)".as_ptr(),
                        c_uint::from(b),
                    );
                    return Dict::new();
                }
                _ => {
                    api_set_error!(
                        err,
                        ErrorType::kErrorTypeValidation,
                        c"Invalid flag: '%c' (%u)".as_ptr(),
                        c_int::from(b as c_char),
                        c_uint::from(b),
                    );
                    return Dict::new();
                }
            }
        }

        let parser_lines = [
            ParserLine {
                data: expr.data,
                size: expr.size,
            },
            ParserLine {
                data: std::ptr::null(),
                size: 0,
            },
        ];
        let mut pstate = ParserState {
            reader: ParserInputReader::new(&parser_lines),
            pos: ParserPosition { line: 0, col: 0 },
            colors: hl.then(ParserHighlight::new),
        };

        let east = viml_pexpr_parse(&mut pstate, pflags);

        let ret_size = 2 + usize::from(east.err.msg.is_some()) + usize::from(hl);
        let mut ret = arena_dict(arena, ret_size);
        let parsed_len = if pstate.pos.line == 1 {
            parser_lines[0].size
        } else {
            pstate.pos.col
        };
        put_c(&mut ret, c"len", integer_obj(parsed_len as Integer));

        if let Some(err_msg) = east.err.msg {
            let mut err_dict = arena_dict(arena, 2);
            let msg_str = arena_string(arena, cstr_as_string(err_msg.as_ptr()));
            put_c(&mut err_dict, c"message", string_obj(msg_str));
            put_c(
                &mut err_dict,
                c"arg",
                cbuf_to_arena_obj(arena, east.err.arg, east.err.arg_len as usize),
            );
            put_c(&mut ret, c"error", dict_obj(err_dict));
        }

        if let Some(colors) = pstate.colors.take() {
            let mut hl_arr = arena_array(arena, colors.len());
            for chunk in &colors {
                let mut chunk_arr = arena_array(arena, 4);
                add_c(&mut chunk_arr, integer_obj(chunk.start.line as Integer));
                add_c(&mut chunk_arr, integer_obj(chunk.start.col as Integer));
                add_c(&mut chunk_arr, integer_obj(chunk.end_col as Integer));
                add_c(&mut chunk_arr, string_obj(cstr_as_string(chunk.group.as_ptr())));
                add_c(&mut hl_arr, array_obj(chunk_arr));
            }
            put_c(&mut ret, c"highlight", array_obj(hl_arr));
        }

        let ast = if let Some(ref root) = east.root {
            expr_ast_node_to_obj(root, arena)
        } else {
            nil_obj()
        };
        put_c(&mut ret, c"ast", ast);

        debug_assert_eq!(ret.size, ret.capacity);
        ret
    }
}

unsafe fn expr_ast_node_to_obj(node: &ExprASTNode, arena: *mut Arena) -> Object {
    unsafe {
        let nt = node.r#type;
        let items_size = 3
            + usize::from(node.children.is_some())
            + usize::from(nt == kExprNodeOption || nt == kExprNodePlainIdentifier)
            + usize::from(
                nt == kExprNodeOption
                    || nt == kExprNodePlainIdentifier
                    || nt == kExprNodePlainKey
                    || nt == kExprNodeEnvironment,
            )
            + usize::from(nt == kExprNodeRegister)
            + 3 * usize::from(nt == kExprNodeComparison)
            + usize::from(nt == kExprNodeInteger)
            + usize::from(nt == kExprNodeFloat)
            + usize::from(nt == kExprNodeDoubleQuotedString || nt == kExprNodeSingleQuotedString)
            + usize::from(nt == kExprNodeAssignment);
        let mut ret_node = arena_dict(arena, items_size);

        if let Some(ref first_child) = node.children {
            let num_children = 1 + usize::from(first_child.next.is_some());
            let mut children_array = arena_array(arena, num_children);
            add_c(
                &mut children_array,
                expr_ast_node_to_obj(first_child, arena),
            );
            if let Some(ref second_child) = first_child.next {
                add_c(
                    &mut children_array,
                    expr_ast_node_to_obj(second_child, arena),
                );
            }
            put_c(&mut ret_node, c"children", array_obj(children_array));
        }

        put_c(
            &mut ret_node,
            c"type",
            string_obj(cstr_as_string(east_node_type_tab[nt as usize].as_ptr())),
        );
        let mut start_array = arena_array(arena, 2);
        add_c(&mut start_array, integer_obj(node.start.line as Integer));
        add_c(&mut start_array, integer_obj(node.start.col as Integer));
        put_c(&mut ret_node, c"start", array_obj(start_array));
        put_c(&mut ret_node, c"len", integer_obj(node.len as Integer));

        match nt {
            kExprNodeDoubleQuotedString | kExprNodeSingleQuotedString => {
                let str_obj =
                    cbuf_to_arena_obj(arena, node.data.str().value_ptr(), node.data.str().size);
                put_c(&mut ret_node, c"svalue", str_obj);
            }
            kExprNodeOption => {
                put_c(
                    &mut ret_node,
                    c"scope",
                    integer_obj(i64::from(node.data.opt().scope)),
                );
                put_c(
                    &mut ret_node,
                    c"ident",
                    cbuf_to_arena_obj(arena, node.data.opt().ident, node.data.opt().ident_len),
                );
            }
            kExprNodePlainIdentifier => {
                put_c(
                    &mut ret_node,
                    c"scope",
                    integer_obj(i64::from(node.data.var().scope)),
                );
                put_c(
                    &mut ret_node,
                    c"ident",
                    cbuf_to_arena_obj(arena, node.data.var().ident, node.data.var().ident_len),
                );
            }
            kExprNodePlainKey => {
                put_c(
                    &mut ret_node,
                    c"ident",
                    cbuf_to_arena_obj(arena, node.data.var().ident, node.data.var().ident_len),
                );
            }
            kExprNodeEnvironment => {
                put_c(
                    &mut ret_node,
                    c"ident",
                    cbuf_to_arena_obj(arena, node.data.env().ident, node.data.env().ident_len),
                );
            }
            kExprNodeRegister => {
                put_c(
                    &mut ret_node,
                    c"name",
                    integer_obj(i64::from(node.data.reg().name)),
                );
            }
            kExprNodeComparison => {
                put_c(
                    &mut ret_node,
                    c"cmp_type",
                    string_obj(cstr_as_string(
                        eltkn_cmp_type_tab[node.data.cmp().r#type as usize].as_ptr(),
                    )),
                );
                put_c(
                    &mut ret_node,
                    c"ccs_strategy",
                    string_obj(cstr_as_string(
                        ccs_tab[node.data.cmp().ccs as usize]
                            .map_or(std::ptr::null(), |s| s.as_ptr()),
                    )),
                );
                put_c(&mut ret_node, c"invert", boolean_obj(node.data.cmp().inv));
            }
            kExprNodeFloat => {
                put_c(&mut ret_node, c"fvalue", float_obj(node.data.flt().value));
            }
            kExprNodeInteger => {
                let v = node.data.num().value;
                let iv = if v > API_INTEGER_MAX {
                    API_INTEGER_MAX as Integer
                } else {
                    v as Integer
                };
                put_c(&mut ret_node, c"ivalue", integer_obj(iv));
            }
            kExprNodeAssignment => {
                let asgn_type = node.data.ass().r#type;
                let str_val = if asgn_type == kExprAsgnPlain {
                    NvimString {
                        data: std::ptr::null_mut(),
                        size: 0,
                    }
                } else {
                    cstr_as_string(expr_asgn_type_tab[asgn_type as usize].as_ptr())
                };
                put_c(&mut ret_node, c"augmentation", string_obj(str_val));
            }
            _ => {}
        }
        debug_assert_eq!(ret_node.size, ret_node.capacity);
        dict_obj(ret_node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cstr_as_string_handles_null_and_non_null() {
        unsafe {
            let empty = cstr_as_string(std::ptr::null());
            assert!(empty.data.is_null());
            assert_eq!(empty.size, 0);

            let s = cstr_as_string(c"echo 1".as_ptr());
            assert_eq!(s.size, 6);
        }
    }
}
