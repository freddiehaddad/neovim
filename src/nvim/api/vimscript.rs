//! Vimscript execution, evaluation, and expression parsing API (`src/nvim/api/vimscript.c`).
#![forbid(unsafe_code)]

use crate::api::private::converter::{object_to_vim, vim_to_object};
use crate::api::private::helpers::{
    TryState, add_c, api_error, api_set_sctx, array_obj, boolean_obj, bytes_to_string,
    cstr_as_string, dict_obj, float_obj, integer_obj, nil_obj, put_c, string_obj, try_enter,
    try_leave,
};
use crate::api::private::validate::{
    Array, Dict, Error, ErrorType, KeyValuePair, NvimString, Object, ObjectType, api_err_exp,
};
use crate::eval::eval0;
use crate::eval::typval::{TvDict, TypVal, VarType, tv_clear};
use crate::eval::userfunc::{Funcexe, call_func};
use crate::ex_docmd::do_cmdline_cmd;
use crate::globals::*;
use crate::lua::converter::Boolean;
use crate::msgpack_rpc::packer::Integer;
use crate::runtime::do_source_str;
use crate::viml::parser::expressions::{
    CCS_TAB, EAST_NODE_TYPE_TAB, ELTKN_CMP_TYPE_TAB, EXPR_ASGN_TYPE_TAB, ExprASTNode,
    K_EXPR_ASGN_PLAIN, K_EXPR_FLAGS_DISALLOW_EOC, K_EXPR_FLAGS_MULTI, K_EXPR_FLAGS_PARSE_LET,
    K_EXPR_NODE_ASSIGNMENT, K_EXPR_NODE_COMPARISON, K_EXPR_NODE_DOUBLE_QUOTED_STRING,
    K_EXPR_NODE_ENVIRONMENT, K_EXPR_NODE_FLOAT, K_EXPR_NODE_INTEGER, K_EXPR_NODE_OPTION,
    K_EXPR_NODE_PLAIN_IDENTIFIER, K_EXPR_NODE_PLAIN_KEY, K_EXPR_NODE_REGISTER,
    K_EXPR_NODE_SINGLE_QUOTED_STRING, viml_pexpr_parse,
};
use crate::viml::parser::parser::{
    ParserHighlight, ParserInputReader, ParserLine, ParserPosition, ParserState,
};

const API_INTEGER_MAX: u64 = i64::MAX as u64;

#[derive(Default)]
pub struct KeyDictExecOpts {
    pub output: Boolean,
}

use crate::eval::funcs::MAX_FUNC_ARGS;

struct ApiVimscriptState {
    eval_recursive: std::cell::Cell<i32>,
    call_func_recursive: std::cell::Cell<i32>,
}

impl ApiVimscriptState {
    const fn new() -> Self {
        Self {
            eval_recursive: std::cell::Cell::new(0),
            call_func_recursive: std::cell::Cell::new(0),
        }
    }
}

std::thread_local! {
    static API_VIMS_STATE: ApiVimscriptState = const { ApiVimscriptState::new() };
}

use crate::api::private::helpers::cstr_to_string;

#[inline]
fn cbuf_to_arena_obj(bytes: &[u8]) -> Object {
    string_obj(NvimString::from_bytes(bytes))
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
pub fn nvim_exec2(
    channel_id: u64,
    src: NvimString,
    opts: &mut KeyDictExecOpts,
) -> Result<Dict, Error> {
    let mut result = Dict::new();
    let output = exec_impl(channel_id, src, &mut *opts)?;
    if opts.output {
        result.push(KeyValuePair {
            key: cstr_to_string(Some(c"output")),
            value: string_obj(output),
        });
    }
    Ok(result)
}

// DLLEXPORT String exec_impl(uint64_t channel_id, String src, Dict(exec_opts) *opts, Error *err);
pub fn exec_impl(
    channel_id: u64,
    src: NvimString,
    opts: &mut KeyDictExecOpts,
) -> Result<NvimString, Error> {
    let save_msg_silent = g_st().msg_silent.get();
    let save_redir_off = g_st().redir_off.get();
    let mut save_capture_ga = g_st().capture_ga.take();
    let save_msg_col = g_st().msg_col.get();
    let mut capture_local: Vec<u8> = Vec::new();
    if opts.output {
        *g_st().capture_ga.borrow_mut() = Some(Vec::new());
    } else {
        *g_st().capture_ga.borrow_mut() = save_capture_ga.take();
    }

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    if opts.output {
        g_st().msg_silent.set(g_st().msg_silent.get() + (1));
        g_st().redir_off.set(false);
        g_st().msg_col.set(0);
    }

    let save_current_sctx = api_set_sctx(channel_id);
    do_source_str(src.as_cstr(), c"nvim_exec2()");
    if opts.output {
        capture_local = g_st()
            .capture_ga
            .replace(save_capture_ga)
            .unwrap_or_default();
        g_st().msg_silent.set(save_msg_silent);
        g_st().redir_off.set(save_redir_off);
        g_st().msg_col.set(save_msg_col);
    }
    g_st().current_sctx.set(save_current_sctx);
    try_leave(&tstate)?;

    if opts.output && capture_local.len() > 1 {
        let slice = capture_local.strip_prefix(b"\n").unwrap_or(&capture_local);
        return Ok(bytes_to_string(slice));
    }

    Ok(NvimString::new())
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
pub fn nvim_command(cmd: NvimString) -> Result<(), Error> {
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    do_cmdline_cmd(cmd.as_cstr());
    try_leave(&tstate)
}

/// Evaluates a Vimscript |expression|. Dicts and Lists are recursively expanded.
///
/// On execution error: fails with Vimscript error, updates v:errmsg.
///
/// @param expr     Vimscript expression string
/// @param[out] err Error details, if any
/// @return         Evaluation result or expanded object
// DLLEXPORT Object nvim_eval(String expr, Arena *arena, Error *err) FUNC_API_SINCE(1);
pub fn nvim_eval(expr: NvimString) -> Result<Object, Error> {
    let eval_rec = API_VIMS_STATE.with(|st| st.eval_recursive.get());
    if eval_rec == 0 {
        g_st().force_abort.set(false);
        g_st().suppress_errthrow.set(false);
        g_st().did_throw.set(false);
        g_st().did_emsg.set(0);
    }
    API_VIMS_STATE.with(|st| st.eval_recursive.set(eval_rec + 1));

    let mut rettv: TypVal = TypVal::new();
    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let ok = eval0(
        if expr.data.is_null() {
            c""
        } else {
            expr.as_cstr()
        },
        &mut rettv,
        None,
        &mut crate::eval::vars::Evalarg { eval_flags: 1 },
    );
    let res = try_leave(&tstate).and_then(|()| {
        if !ok {
            Err(api_error!(
                ErrorType::ErrorTypeException,
                c"Failed to evaluate expression: '%.*s'",
                256_i32,
                expr,
            ))
        } else {
            Ok(vim_to_object(&rettv))
        }
    });

    tv_clear(&mut rettv);
    API_VIMS_STATE.with(|st| st.eval_recursive.set(st.eval_recursive.get() - 1));

    res
}

fn call_function_impl(
    channel_id: u64,
    fn_name: NvimString,
    args: Array,
    self_dict: Option<&TvDict>,
) -> Result<Object, Error> {
    if args.size > MAX_FUNC_ARGS {
        return Err(api_error!(
            ErrorType::ErrorTypeValidation,
            c"Function called with too many arguments",
        ));
    }

    let mut vim_args: [TypVal; MAX_FUNC_ARGS + 1] = [const { TypVal::new() }; MAX_FUNC_ARGS + 1];
    let mut i = 0usize;
    while i < args.size {
        object_to_vim(args[i].clone(), &mut vim_args[i]);
        i += 1;
    }

    let call_rec = API_VIMS_STATE.with(|st| st.call_func_recursive.get());
    if call_rec == 0 {
        g_st().force_abort.set(false);
        g_st().suppress_errthrow.set(false);
        g_st().did_throw.set(false);
        g_st().did_emsg.set(0);
    }
    API_VIMS_STATE.with(|st| st.call_func_recursive.set(call_rec + 1));

    let mut rettv: TypVal = TypVal::new();
    let lnum = curwin().w_cursor.get().lnum;
    let mut funcexe = Funcexe {
        fe_fill_submatch: false,
        fe_firstline: lnum,
        fe_lastline: lnum,
        fe_doesrange: false,
        fe_evaluate: true,
        fe_partial: crate::eval::typval::PartialId::NULL,
        fe_selfdict: crate::eval::typval::DictId::from(self_dict),
        fe_basetv: None,
        fe_found_var: false,
    };

    let mut tstate = TryState::new();
    try_enter(&mut tstate);
    let save_current_sctx = api_set_sctx(channel_id);
    let _ = call_func(
        fn_name.as_cstr(),
        fn_name.size as i32,
        &mut rettv,
        args.size as i32,
        &mut vim_args,
        &mut funcexe,
    );
    g_st().current_sctx.set(save_current_sctx);
    let res = try_leave(&tstate).map(|()| vim_to_object(&rettv));

    tv_clear(&mut rettv);
    API_VIMS_STATE.with(|st| st.call_func_recursive.set(st.call_func_recursive.get() - 1));

    while i > 0 {
        i -= 1;
        tv_clear(&mut vim_args[i]);
    }

    res
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
pub fn nvim_call_function(
    channel_id: u64,
    fn_name: NvimString,
    args: Array,
) -> Result<Object, Error> {
    call_function_impl(channel_id, fn_name, args, None)
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
pub fn nvim_call_dict_function(
    channel_id: u64,
    dict: Object,
    mut fn_name: NvimString,
    args: Array,
) -> Result<Object, Error> {
    let mut rettv: TypVal = TypVal::new();
    let mut mustfree = false;

    match dict.obj_type {
        ObjectType::ObjectTypeString => {
            let mut tstate = TryState::new();
            try_enter(&mut tstate);
            let s_val = dict.data.string();
            let eval_ret = eval0(
                s_val.as_cstr(),
                &mut rettv,
                None,
                &mut crate::eval::vars::Evalarg { eval_flags: 1 },
            );
            try_leave(&tstate)?;
            if !eval_ret {
                std::process::abort();
            }
            mustfree = true;
        }
        ObjectType::ObjectTypeDict => {
            object_to_vim(dict.clone(), &mut rettv);
        }
        _ => {
            return Err(api_err_exp(c"dict argument", c"String or Dict", None));
        }
    }

    let fn_name_owned;
    let res = 'body: {
        let Some(self_dict_ref) = rettv.vval.v_dict_opt() else {
            break 'body Err(api_error!(
                ErrorType::ErrorTypeValidation,
                c"dict not found",
            ));
        };
        if rettv.v_type != VarType::VarDict {
            break 'body Err(api_error!(
                ErrorType::ErrorTypeValidation,
                c"dict not found",
            ));
        }

        if !fn_name.data.is_null()
            && fn_name.size > 0
            && dict.obj_type != ObjectType::ObjectTypeDict
        {
            let Some(di) = crate::eval::typval::tv_dict_find_opt(
                Some(self_dict_ref),
                fn_name.as_cstr(),
                fn_name.size as isize,
            ) else {
                break 'body Err(api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"Not found: %s",
                    fn_name,
                ));
            };
            let di_tv = di.tv();
            if di_tv.v_type == VarType::VarPartial {
                break 'body Err(api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"partial function not supported",
                ));
            }
            if di_tv.v_type != VarType::VarFunc {
                break 'body Err(api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"Not a function: %s",
                    fn_name,
                ));
            }
            fn_name_owned = di_tv.vval.v_string_cstr().map(std::ffi::CStr::to_owned);
            drop(di_tv);
            fn_name = cstr_as_string(fn_name_owned.as_deref());
        }

        if fn_name.data.is_null() || fn_name.size == 0 {
            break 'body Err(api_error!(
                ErrorType::ErrorTypeValidation,
                c"Invalid function name: %s",
                c"(empty)",
            ));
        }

        call_function_impl(channel_id, fn_name, args, Some(self_dict_ref))
    };

    if mustfree {
        tv_clear(&mut rettv);
    }

    res
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
pub fn nvim_parse_expression(
    expr: NvimString,
    flags: NvimString,
    hl: Boolean,
) -> Result<Dict, Error> {
    let mut pflags: i32 = 0;
    for &b in flags.as_bytes() {
        match b {
            b'm' => pflags |= K_EXPR_FLAGS_MULTI,
            b'E' => pflags |= K_EXPR_FLAGS_DISALLOW_EOC,
            b'l' => pflags |= K_EXPR_FLAGS_PARSE_LET,
            0 => {
                return Err(api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"Invalid flag: '\\0' (%u)",
                    u32::from(b),
                ));
            }
            _ => {
                return Err(api_error!(
                    ErrorType::ErrorTypeValidation,
                    c"Invalid flag: '%c' (%u)",
                    i32::from(b as i8),
                    u32::from(b),
                ));
            }
        }
    }

    let parser_lines = [ParserLine::from_slice(expr.as_bytes()), ParserLine::NONE];
    let mut pstate = ParserState {
        reader: ParserInputReader::new(&parser_lines),
        pos: ParserPosition { line: 0, col: 0 },
        colors: hl.then(ParserHighlight::new),
    };

    let east = viml_pexpr_parse(&mut pstate, pflags);

    let ret_size = 2 + usize::from(east.err.msg.is_some()) + usize::from(hl);
    let mut ret = Dict::with_capacity(ret_size);
    let parsed_len = if pstate.pos.line == 1 {
        parser_lines[0].size()
    } else {
        pstate.pos.col
    };
    put_c(&mut ret, c"len", integer_obj(parsed_len as Integer));

    if let Some(err_msg) = east.err.msg {
        let mut err_dict = Dict::with_capacity(2);
        let msg_str =
            crate::api::private::helpers::ensure_nul_string(cstr_as_string(Some(err_msg)));
        put_c(&mut err_dict, c"message", string_obj(msg_str));
        put_c(
            &mut err_dict,
            c"arg",
            cbuf_to_arena_obj(east.err.arg_bytes()),
        );
        put_c(&mut ret, c"error", dict_obj(err_dict));
    }

    if let Some(colors) = pstate.colors.take() {
        let mut hl_arr = Array::with_capacity(colors.len());
        for chunk in &colors {
            let mut chunk_arr = Array::with_capacity(4);
            add_c(&mut chunk_arr, integer_obj(chunk.start.line as Integer));
            add_c(&mut chunk_arr, integer_obj(chunk.start.col as Integer));
            add_c(&mut chunk_arr, integer_obj(chunk.end_col as Integer));
            add_c(
                &mut chunk_arr,
                string_obj(cstr_as_string(Some(chunk.group))),
            );
            add_c(&mut hl_arr, array_obj(chunk_arr));
        }
        put_c(&mut ret, c"highlight", array_obj(hl_arr));
    }

    let ast = if let Some(ref root) = east.root {
        expr_ast_node_to_obj(root)
    } else {
        nil_obj()
    };
    put_c(&mut ret, c"ast", ast);

    debug_assert_eq!(ret.size, ret.capacity);
    Ok(ret)
}

fn expr_ast_node_to_obj(node: &ExprASTNode) -> Object {
    let nt = node.r#type;
    let items_size = 3
        + usize::from(node.children.is_some())
        + usize::from(nt == K_EXPR_NODE_OPTION || nt == K_EXPR_NODE_PLAIN_IDENTIFIER)
        + usize::from(
            nt == K_EXPR_NODE_OPTION
                || nt == K_EXPR_NODE_PLAIN_IDENTIFIER
                || nt == K_EXPR_NODE_PLAIN_KEY
                || nt == K_EXPR_NODE_ENVIRONMENT,
        )
        + usize::from(nt == K_EXPR_NODE_REGISTER)
        + 3 * usize::from(nt == K_EXPR_NODE_COMPARISON)
        + usize::from(nt == K_EXPR_NODE_INTEGER)
        + usize::from(nt == K_EXPR_NODE_FLOAT)
        + usize::from(
            nt == K_EXPR_NODE_DOUBLE_QUOTED_STRING || nt == K_EXPR_NODE_SINGLE_QUOTED_STRING,
        )
        + usize::from(nt == K_EXPR_NODE_ASSIGNMENT);
    let mut ret_node = Dict::with_capacity(items_size);

    if let Some(ref first_child) = node.children {
        let num_children = 1 + usize::from(first_child.next.is_some());
        let mut children_array = Array::with_capacity(num_children);
        add_c(&mut children_array, expr_ast_node_to_obj(first_child));
        if let Some(ref second_child) = first_child.next {
            add_c(&mut children_array, expr_ast_node_to_obj(second_child));
        }
        put_c(&mut ret_node, c"children", array_obj(children_array));
    }

    put_c(
        &mut ret_node,
        c"type",
        string_obj(cstr_as_string(Some(EAST_NODE_TYPE_TAB[nt as usize]))),
    );
    let mut start_array = Array::with_capacity(2);
    add_c(&mut start_array, integer_obj(node.start.line as Integer));
    add_c(&mut start_array, integer_obj(node.start.col as Integer));
    put_c(&mut ret_node, c"start", array_obj(start_array));
    put_c(&mut ret_node, c"len", integer_obj(node.len as Integer));

    match nt {
        K_EXPR_NODE_DOUBLE_QUOTED_STRING | K_EXPR_NODE_SINGLE_QUOTED_STRING => {
            let str_obj = cbuf_to_arena_obj(node.data.str().as_bytes());
            put_c(&mut ret_node, c"svalue", str_obj);
        }
        K_EXPR_NODE_OPTION => {
            put_c(
                &mut ret_node,
                c"scope",
                integer_obj(i64::from(node.data.opt().scope)),
            );
            put_c(
                &mut ret_node,
                c"ident",
                cbuf_to_arena_obj(node.data.opt().ident_bytes()),
            );
        }
        K_EXPR_NODE_PLAIN_IDENTIFIER => {
            put_c(
                &mut ret_node,
                c"scope",
                integer_obj(i64::from(node.data.var().scope)),
            );
            put_c(
                &mut ret_node,
                c"ident",
                cbuf_to_arena_obj(node.data.var().ident_bytes()),
            );
        }
        K_EXPR_NODE_PLAIN_KEY => {
            put_c(
                &mut ret_node,
                c"ident",
                cbuf_to_arena_obj(node.data.var().ident_bytes()),
            );
        }
        K_EXPR_NODE_ENVIRONMENT => {
            put_c(
                &mut ret_node,
                c"ident",
                cbuf_to_arena_obj(node.data.env().ident_bytes()),
            );
        }
        K_EXPR_NODE_REGISTER => {
            put_c(
                &mut ret_node,
                c"name",
                integer_obj(i64::from(node.data.reg().name)),
            );
        }
        K_EXPR_NODE_COMPARISON => {
            put_c(
                &mut ret_node,
                c"cmp_type",
                string_obj(cstr_as_string(Some(
                    ELTKN_CMP_TYPE_TAB[node.data.cmp().r#type as usize],
                ))),
            );
            put_c(
                &mut ret_node,
                c"ccs_strategy",
                string_obj(cstr_as_string(CCS_TAB[node.data.cmp().ccs as usize])),
            );
            put_c(&mut ret_node, c"invert", boolean_obj(node.data.cmp().inv));
        }
        K_EXPR_NODE_FLOAT => {
            put_c(&mut ret_node, c"fvalue", float_obj(node.data.flt().value));
        }
        K_EXPR_NODE_INTEGER => {
            let v = node.data.num().value;
            let iv = if v > API_INTEGER_MAX {
                API_INTEGER_MAX as Integer
            } else {
                v as Integer
            };
            put_c(&mut ret_node, c"ivalue", integer_obj(iv));
        }
        K_EXPR_NODE_ASSIGNMENT => {
            let asgn_type = node.data.ass().r#type;
            let str_val = if asgn_type == K_EXPR_ASGN_PLAIN {
                NvimString::new()
            } else {
                cstr_as_string(Some(EXPR_ASGN_TYPE_TAB[asgn_type as usize]))
            };
            put_c(&mut ret_node, c"augmentation", string_obj(str_val));
        }
        _ => {}
    }
    debug_assert_eq!(ret_node.size, ret_node.capacity);
    dict_obj(ret_node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cstr_as_string_handles_null_and_non_null() {
        let empty = cstr_as_string(None);
        assert!(empty.data.is_null());
        assert_eq!(empty.size, 0);

        let s = cstr_as_string(Some(c"echo 1"));
        assert_eq!(s.size, 6);
    }
}
