//! Functions for text objects (`src/nvim/textobject.c`).

use crate::charset::{NUL, ascii_iswhite, skipwhite};
use crate::cursor::{
    clearpos, coladvance, colnr_T, dec_cursor, equalpos, gchar_cursor, get_cursor_line_ptr,
    get_cursor_pos_ptr, inc_cursor, linenr_T, lt, ltoreq, pos_T,
};
use crate::drawscreen::{UPD_INVERTED, redraw_curbuf_later, showmode};
use crate::eval::funcs::{do_searchpair, kMTCharWise, kMTLineWise};
use crate::fold::hasFolding;
use crate::globals::*;
use crate::indent::inindent;
use crate::insert::oneleft;
use crate::mark::setpcmark;
use crate::mbyte::{mb_prevptr, utf_class, utf_head_off, utfc_ptr2len};
use crate::memline::{dec, decl, gchar_pos, inc, incl, ml_get, ml_get_len, ml_get_pos};
use crate::r#move::adjust_skipcol;
use crate::normal::{FM_FORWARD, lineempty, unadjust_for_sel};
use crate::option_vars::*;
use crate::search::{
    BACKWARD, Direction, FORWARD, findmatch, findmatchlimit, kCpoMatchbsl, linewhite,
};
use crate::strings::vim_strchr;
use crate::textformat::oparg_T;
use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use crate::math::{FAIL, OK};
use crate::regexp::MAXCOL;

const kCpoEndofsent: c_int = b'J' as c_int;

struct TextobjectState {
    CLS_BIGWORD: bool,
}

impl TextobjectState {
    const fn new() -> Self {
        Self {
            CLS_BIGWORD: false,
        }
    }
}

static TOBJ_STATE: crate::globals::GlobalState<TextobjectState> = crate::globals::GlobalState::new(TextobjectState::new());

#[inline]
unsafe fn tobj_st() -> &'static mut TextobjectState {
    unsafe { &mut *TOBJ_STATE.as_ptr() }
}

/// Find the start of the next sentence, searching in the direction specified
/// by the `dir` argument (`findsent`).
pub unsafe fn findsent(dir: Direction, mut count: c_int) -> c_int {
    unsafe {
        let mut c: c_int;
        let func: unsafe fn(*mut pos_T) -> c_int = if dir == FORWARD { incl } else { decl };
        let mut noskip = false;

        let mut pos = (*curwin).w_cursor;

        while {
            let old = count;
            count -= 1;
            old != 0
        } {
            let prev_pos = pos;
            let mut goto_found = false;

            // if on an empty line, skip up to a non-empty line
            if gchar_pos(&raw mut pos) == c_int::from(NUL) {
                loop {
                    if func(&raw mut pos) == -1 {
                        break;
                    }
                    if gchar_pos(&raw mut pos) != c_int::from(NUL) {
                        break;
                    }
                }
                if dir == FORWARD {
                    goto_found = true;
                }
            } else if dir == FORWARD && pos.col == 0 && startPS(pos.lnum, c_int::from(NUL), false) {
                // if on the start of a paragraph or a section and searching forward,
                // go to the next line
                if pos.lnum == (*curbuf).b_ml.ml_line_count {
                    return FAIL;
                }
                pos.lnum += 1;
                goto_found = true;
            } else if dir == BACKWARD {
                decl(&raw mut pos);
            }

            if !goto_found {
                // go back to the previous non-white non-punctuation character
                let mut found_dot = false;
                while {
                    c = gchar_pos(&raw mut pos);
                    ascii_iswhite(c) || !vim_strchr(c".!?)]\"'".as_ptr(), c).is_null()
                } {
                    let mut tpos = pos;
                    if decl(&raw mut tpos) == -1 || (lineempty(tpos.lnum) && dir == FORWARD) {
                        break;
                    }
                    if found_dot {
                        break;
                    }
                    if !vim_strchr(c".!?".as_ptr(), c).is_null() {
                        found_dot = true;
                    }
                    if !vim_strchr(c")]\"'".as_ptr(), c).is_null()
                        && vim_strchr(c".!?)]\"'".as_ptr(), gchar_pos(&raw mut tpos)).is_null()
                    {
                        break;
                    }
                    decl(&raw mut pos);
                }

                // remember the line where the search started
                let startlnum = pos.lnum;
                let cpo_J = !vim_strchr(p_cpo, kCpoEndofsent).is_null();

                loop {
                    c = gchar_pos(&raw mut pos);
                    if c == c_int::from(NUL)
                        || (pos.col == 0 && startPS(pos.lnum, c_int::from(NUL), false))
                    {
                        if dir == BACKWARD && pos.lnum != startlnum {
                            pos.lnum += 1;
                        }
                        break;
                    }
                    if c == b'.' as c_int || c == b'!' as c_int || c == b'?' as c_int {
                        let mut tpos = pos;
                        loop {
                            c = inc(&raw mut tpos);
                            if c == -1 {
                                break;
                            }
                            c = gchar_pos(&raw mut tpos);
                            if vim_strchr(c")]\"'".as_ptr(), c).is_null() {
                                break;
                            }
                        }
                        if c == -1
                            || (!cpo_J && (c == b' ' as c_int || c == b'\t' as c_int))
                            || c == c_int::from(NUL)
                            || (cpo_J
                                && c == b' ' as c_int
                                && inc(&raw mut tpos) >= 0
                                && gchar_pos(&raw mut tpos) == b' ' as c_int)
                        {
                            pos = tpos;
                            if gchar_pos(&raw mut pos) == c_int::from(NUL) {
                                inc(&raw mut pos);
                            }
                            break;
                        }
                    }
                    if func(&raw mut pos) == -1 {
                        if count != 0 {
                            return FAIL;
                        }
                        noskip = true;
                        break;
                    }
                }
            }

            // skip white space
            while !noskip && {
                c = gchar_pos(&raw mut pos);
                c == b' ' as c_int || c == b'\t' as c_int
            } {
                if incl(&raw mut pos) == -1 {
                    break;
                }
            }

            if equalpos(prev_pos, pos) {
                // didn't actually move, advance one character and try again
                if func(&raw mut pos) == -1 {
                    if count != 0 {
                        return FAIL;
                    }
                    break;
                }
                count += 1;
            }
        }

        setpcmark();
        (*curwin).w_cursor = pos;
        OK
    }
}

/// Find the next paragraph or section in direction `dir` (`findpar`).
pub unsafe fn findpar(
    pincl: *mut bool,
    dir: c_int,
    mut count: c_int,
    what: c_int,
    both: bool,
) -> bool {
    unsafe {
        let mut fold_first: linenr_T = 0;
        let mut fold_last: linenr_T = 0;
        let mut curr = (*curwin).w_cursor.lnum;

        while {
            let old = count;
            count -= 1;
            old != 0
        } {
            let mut did_skip = false;
            let mut first = true;
            loop {
                if *ml_get(curr) != NUL {
                    did_skip = true;
                }

                // skip folded lines
                let mut fold_skipped = false;
                if first && hasFolding(curwin, curr, &raw mut fold_first, &raw mut fold_last) {
                    curr = (if dir > 0 { fold_last } else { fold_first }) + dir;
                    fold_skipped = true;
                }

                if !first && did_skip && startPS(curr, what, both) {
                    break;
                }

                if fold_skipped {
                    curr -= dir;
                }
                curr += dir;
                if curr < 1 || curr > (*curbuf).b_ml.ml_line_count {
                    if count != 0 {
                        return false;
                    }
                    curr -= dir;
                    break;
                }
                first = false;
            }
        }
        setpcmark();
        if both && *ml_get(curr) == b'}' as c_char {
            curr += 1;
        }
        (*curwin).w_cursor.lnum = curr;
        if curr == (*curbuf).b_ml.ml_line_count && what != b'}' as c_int && dir == FORWARD {
            let line = ml_get(curr);

            // Put the cursor on the last character in the last line and make the
            // motion inclusive.
            (*curwin).w_cursor.col = ml_get_len(curr);
            if (*curwin).w_cursor.col != 0 {
                (*curwin).w_cursor.col -= 1;
                (*curwin).w_cursor.col -=
                    utf_head_off(line, line.add((*curwin).w_cursor.col as usize));
                *pincl = true;
            }
        } else {
            (*curwin).w_cursor.col = 0;
        }
        true
    }
}

/// Check if the string `s` is a nroff macro that is in option `opt` (`inmacro`).
unsafe fn inmacro(opt: *mut c_char, s: *const c_char) -> bool {
    unsafe {
        let mut macro_p = opt;
        let s0 = *s;
        let s1 = if s0 == NUL { NUL } else { *s.add(1) };

        while *macro_p != NUL {
            let m0 = *macro_p;
            let m1 = *macro_p.add(1);
            if (m0 == s0 || (m0 == b' ' as c_char && (s0 == NUL || s0 == b' ' as c_char)))
                && (m1 == s1
                    || ((m1 == NUL || m1 == b' ' as c_char)
                        && (s0 == NUL || s1 == NUL || s1 == b' ' as c_char)))
            {
                break;
            }
            macro_p = macro_p.add(1);
            if *macro_p == NUL {
                break;
            }
            macro_p = macro_p.add(1);
        }
        *macro_p != NUL
    }
}

/// Return true if line `lnum` is the start of a section or paragraph (`startPS`).
pub unsafe fn startPS(lnum: linenr_T, para: c_int, both: bool) -> bool {
    unsafe {
        let s = ml_get(lnum);
        if c_int::from(*s as u8) == para
            || *s == b'\x0c' as c_char
            || (both && *s == b'}' as c_char)
        {
            return true;
        }
        if *s == b'.' as c_char
            && (inmacro(p_sections, s.add(1)) || (para == 0 && inmacro(p_para, s.add(1))))
        {
            return true;
        }
        false
    }
}

/// Return the class of character at `curwin->w_cursor` (`cls`).
unsafe fn cls() -> c_int {
    unsafe {
        let mut c = gchar_cursor();
        if c == b' ' as c_int || c == b'\t' as c_int || c == c_int::from(NUL) {
            return 0;
        }

        c = utf_class(c);

        if c != 0 && tobj_st().CLS_BIGWORD {
            return 1;
        }
        c
    }
}

/// Move forward `count` words (`fwd_word`).
pub unsafe fn fwd_word(mut count: c_int, bigword: bool, eol: bool) -> c_int {
    unsafe {
        (*curwin).w_cursor.coladd = 0;
        tobj_st().CLS_BIGWORD = bigword;
        while {
            count -= 1;
            count >= 0
        } {
            if hasFolding(
                curwin,
                (*curwin).w_cursor.lnum,
                ptr::null_mut(),
                &raw mut (*curwin).w_cursor.lnum,
            ) {
                coladvance(curwin, MAXCOL);
            }
            let sclass = cls();

            let last_line = (*curwin).w_cursor.lnum == (*curbuf).b_ml.ml_line_count;
            let mut i = inc_cursor();
            if i == -1 || (i >= 1 && last_line) {
                return FAIL;
            }
            if i >= 1 && eol && count == 0 {
                return OK;
            }

            if sclass != 0 {
                while cls() == sclass {
                    i = inc_cursor();
                    if i == -1 || (i >= 1 && eol && count == 0) {
                        return OK;
                    }
                }
            }

            while cls() == 0 {
                if (*curwin).w_cursor.col == 0 && *get_cursor_line_ptr() == NUL {
                    break;
                }

                i = inc_cursor();
                if i == -1 || (i >= 1 && eol && count == 0) {
                    return OK;
                }
            }
        }
        OK
    }
}

/// Move backward `count` words (`bck_word`).
pub unsafe fn bck_word(mut count: c_int, bigword: bool, mut stop: bool) -> c_int {
    unsafe {
        (*curwin).w_cursor.coladd = 0;
        tobj_st().CLS_BIGWORD = bigword;
        while {
            count -= 1;
            count >= 0
        } {
            if hasFolding(
                curwin,
                (*curwin).w_cursor.lnum,
                &raw mut (*curwin).w_cursor.lnum,
                ptr::null_mut(),
            ) {
                (*curwin).w_cursor.col = 0;
            }
            let sclass = cls();
            if dec_cursor() == -1 {
                return FAIL;
            }

            let mut finished = false;
            if !stop || sclass == cls() || sclass == 0 {
                while cls() == 0 {
                    if (*curwin).w_cursor.col == 0 && lineempty((*curwin).w_cursor.lnum) {
                        finished = true;
                        break;
                    }
                    if dec_cursor() == -1 {
                        return OK;
                    }
                }

                if !finished && skip_chars(cls(), BACKWARD) {
                    return OK;
                }
            }

            if !finished {
                inc_cursor();
            }
            stop = false;
        }
        adjust_skipcol();
        OK
    }
}

/// Move to the end of the word (`end_word`).
pub unsafe fn end_word(mut count: c_int, bigword: bool, mut stop: bool, empty: bool) -> c_int {
    unsafe {
        (*curwin).w_cursor.coladd = 0;
        tobj_st().CLS_BIGWORD = bigword;

        if *p_sel == b'e' as c_char
            && Visual.active
            && Visual.mode == b'v' as c_int
            && Visual.select_exclu_adj
        {
            unadjust_for_sel();
        }

        while {
            count -= 1;
            count >= 0
        } {
            if hasFolding(
                curwin,
                (*curwin).w_cursor.lnum,
                ptr::null_mut(),
                &raw mut (*curwin).w_cursor.lnum,
            ) {
                coladvance(curwin, MAXCOL);
            }
            let sclass = cls();
            if inc_cursor() == -1 {
                return FAIL;
            }

            let mut finished = false;
            if cls() == sclass && sclass != 0 {
                if skip_chars(sclass, FORWARD) {
                    return FAIL;
                }
            } else if !stop || sclass == 0 {
                while cls() == 0 {
                    if empty && (*curwin).w_cursor.col == 0 && lineempty((*curwin).w_cursor.lnum) {
                        finished = true;
                        break;
                    }
                    if inc_cursor() == -1 {
                        return FAIL;
                    }
                }

                if !finished && skip_chars(cls(), FORWARD) {
                    return FAIL;
                }
            }
            if !finished {
                dec_cursor();
            }
            stop = false;
        }
        OK
    }
}

/// Move back to the end of the word (`bckend_word`).
pub unsafe fn bckend_word(mut count: c_int, bigword: bool, eol: bool) -> c_int {
    unsafe {
        (*curwin).w_cursor.coladd = 0;
        tobj_st().CLS_BIGWORD = bigword;
        while {
            count -= 1;
            count >= 0
        } {
            let sclass = cls();
            let mut i = dec_cursor();
            if i == -1 {
                return FAIL;
            }
            if eol && i == 1 {
                return OK;
            }

            if sclass != 0 {
                while cls() == sclass {
                    i = dec_cursor();
                    if i == -1 || (eol && i == 1) {
                        return OK;
                    }
                }
            }

            while cls() == 0 {
                if (*curwin).w_cursor.col == 0 && lineempty((*curwin).w_cursor.lnum) {
                    break;
                }
                i = dec_cursor();
                if i == -1 || (eol && i == 1) {
                    return OK;
                }
            }
        }
        adjust_skipcol();
        OK
    }
}

/// Skip a row of characters of the same class (`skip_chars`).
unsafe fn skip_chars(cclass: c_int, dir: Direction) -> bool {
    unsafe {
        while cls() == cclass {
            if (if dir == FORWARD {
                inc_cursor()
            } else {
                dec_cursor()
            }) == -1
            {
                return true;
            }
        }
        false
    }
}

/// Go back to the start of the word or the start of white space (`back_in_line`).
unsafe fn back_in_line() {
    unsafe {
        let sclass = cls();
        loop {
            if (*curwin).w_cursor.col == 0 {
                break;
            }
            dec_cursor();
            if cls() != sclass {
                inc_cursor();
                break;
            }
        }
    }
}

unsafe fn find_first_blank(posp: *mut pos_T) {
    unsafe {
        while decl(posp) != -1 {
            let c = gchar_pos(posp);
            if !ascii_iswhite(c) {
                incl(posp);
                break;
            }
        }
    }
}

/// Skip `count/2` sentences and `count/2` separating white spaces (`findsent_forward`).
unsafe fn findsent_forward(mut count: c_int, mut at_start_sent: bool) {
    unsafe {
        while {
            let old = count;
            count -= 1;
            old != 0
        } {
            findsent(FORWARD, 1);
            if at_start_sent {
                find_first_blank(&raw mut (*curwin).w_cursor);
            }
            if count == 0 || at_start_sent {
                decl(&raw mut (*curwin).w_cursor);
            }
            at_start_sent = !at_start_sent;
        }
    }
}

/// Find word under cursor, cursor at end (`current_word`).
pub unsafe fn current_word(
    oap: *mut oparg_T,
    mut count: c_int,
    include: bool,
    bigword: bool,
) -> c_int {
    unsafe {
        let mut start_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut include_white = false;

        tobj_st().CLS_BIGWORD = bigword;
        clearpos(&mut start_pos);

        if Visual.active && *p_sel == b'e' as c_char && lt(Visual.start, (*curwin).w_cursor) {
            dec_cursor();
        }

        if !Visual.active || equalpos((*curwin).w_cursor, Visual.start) {
            back_in_line();
            start_pos = (*curwin).w_cursor;

            if (cls() == 0) == include {
                if end_word(1, bigword, true, true) == FAIL {
                    return FAIL;
                }
            } else {
                fwd_word(1, bigword, true);
                if (*curwin).w_cursor.col == 0 {
                    decl(&raw mut (*curwin).w_cursor);
                } else {
                    oneleft();
                }

                if include {
                    include_white = true;
                }
            }

            if Visual.active {
                Visual.start = start_pos;
                redraw_curbuf_later(UPD_INVERTED);
            } else {
                (*oap).start = start_pos;
                (*oap).motion_type = kMTCharWise;
            }
            count -= 1;
        }

        while count > 0 {
            inclusive = true;
            if Visual.active && lt((*curwin).w_cursor, Visual.start) {
                if decl(&raw mut (*curwin).w_cursor) == -1 {
                    return FAIL;
                }
                if include != (cls() != 0) {
                    if bck_word(1, bigword, true) == FAIL {
                        return FAIL;
                    }
                } else {
                    if bckend_word(1, bigword, true) == FAIL {
                        return FAIL;
                    }
                    let _ = incl(&raw mut (*curwin).w_cursor);
                }
            } else {
                if incl(&raw mut (*curwin).w_cursor) == -1 {
                    return FAIL;
                }
                if include != (cls() == 0) {
                    if fwd_word(1, bigword, true) == FAIL && count > 1 {
                        return FAIL;
                    }
                    if oneleft() == FAIL {
                        inclusive = false;
                    }
                } else if end_word(1, bigword, true, true) == FAIL {
                    return FAIL;
                }
            }
            count -= 1;
        }

        if include_white && (cls() != 0 || ((*curwin).w_cursor.col == 0 && !inclusive)) {
            let pos = (*curwin).w_cursor;
            (*curwin).w_cursor = start_pos;
            if oneleft() == OK {
                back_in_line();
                if cls() == 0 && (*curwin).w_cursor.col > 0 {
                    if Visual.active {
                        Visual.start = (*curwin).w_cursor;
                    } else {
                        (*oap).start = (*curwin).w_cursor;
                    }
                }
            }
            (*curwin).w_cursor = pos;
        }

        if Visual.active {
            if *p_sel == b'e' as c_char && inclusive && ltoreq(Visual.start, (*curwin).w_cursor) {
                inc_cursor();
            }
            if Visual.mode == b'V' as c_int {
                Visual.mode = b'v' as c_int;
                redraw_cmdline = true;
            }
        } else {
            (*oap).inclusive = inclusive;
        }

        OK
    }
}

/// Find the current line ("il") or all lines ("al") (`current_line`).
pub unsafe fn current_line(oap: *mut oparg_T, include: bool) -> c_int {
    unsafe {
        if include {
            if Visual.active {
                Visual.start.lnum = 1;
                Visual.start.col = 0;
                Visual.start.coladd = 0;
                Visual.mode = b'V' as c_int;
                redraw_curbuf_later(UPD_INVERTED);
                showmode();
            } else {
                (*oap).cursor_start = (*curwin).w_cursor;
                (*oap).restore_cursor = true;
                (*oap).start.lnum = 1;
                (*oap).start.col = 0;
                (*oap).start.coladd = 0;
                (*oap).motion_type = kMTLineWise;
            }
            (*curwin).w_cursor.lnum = (*curbuf).b_ml.ml_line_count;
            (*curwin).w_cursor.col = 0;
            (*curwin).w_cursor.coladd = 0;
            return OK;
        }

        let line = ml_get((*curwin).w_cursor.lnum);
        let start = skipwhite(line);
        let mut end = line.add(CStr::from_ptr(line).to_bytes().len());
        while end > start {
            let prev = mb_prevptr(line, end);
            if !ascii_iswhite(c_int::from(*prev as u8)) {
                break;
            }
            end = prev;
        }
        if start == end {
            return FAIL;
        }

        let mut start_pos = (*curwin).w_cursor;
        start_pos.col = start.offset_from(line) as colnr_T;
        start_pos.coladd = 0;

        let mut end_pos = (*curwin).w_cursor;
        end_pos.col = mb_prevptr(line, end).offset_from(line) as colnr_T;
        end_pos.coladd = 0;

        if Visual.active {
            Visual.start = start_pos;
            (*curwin).w_cursor = end_pos;
            if *p_sel == b'e' as c_char && ltoreq(Visual.start, (*curwin).w_cursor) {
                inc_cursor();
            }
            Visual.mode = b'v' as c_int;
            redraw_curbuf_later(UPD_INVERTED);
            showmode();
        } else {
            (*oap).start = start_pos;
            (*oap).motion_type = kMTCharWise;
            (*oap).inclusive = true;
            (*curwin).w_cursor = end_pos;
        }

        OK
    }
}

/// Find sentence(s) under the cursor, cursor at end (`current_sent`).
pub unsafe fn current_sent(oap: *mut oparg_T, mut count: c_int, include: bool) -> c_int {
    unsafe {
        let mut start_pos = (*curwin).w_cursor;
        let mut pos = start_pos;
        findsent(FORWARD, 1);

        let do_extend = Visual.active && !equalpos(start_pos, Visual.start);
        if !do_extend {
            while ascii_iswhite(gchar_pos(&raw mut pos)) {
                incl(&raw mut pos);
            }
            let start_blank = if equalpos(pos, (*curwin).w_cursor) {
                find_first_blank(&raw mut start_pos);
                true
            } else {
                findsent(BACKWARD, 1);
                start_pos = (*curwin).w_cursor;
                false
            };

            let mut ncount = if include {
                count * 2
            } else {
                let mut n = count;
                if start_blank {
                    n -= 1;
                }
                n
            };
            if ncount > 0 {
                findsent_forward(ncount, true);
            } else {
                decl(&raw mut (*curwin).w_cursor);
            }

            if include {
                if start_blank {
                    find_first_blank(&raw mut (*curwin).w_cursor);
                    let c = gchar_pos(&raw mut (*curwin).w_cursor);
                    if ascii_iswhite(c) {
                        decl(&raw mut (*curwin).w_cursor);
                    }
                } else if !ascii_iswhite(gchar_cursor()) {
                    find_first_blank(&raw mut start_pos);
                }
            }

            if Visual.active {
                if !equalpos(start_pos, (*curwin).w_cursor) {
                    if *p_sel == b'e' as c_char {
                        (*curwin).w_cursor.col += 1;
                    }
                    Visual.start = start_pos;
                    Visual.mode = b'v' as c_int;
                    redraw_cmdline = true;
                    redraw_curbuf_later(UPD_INVERTED);
                    return OK;
                }
            } else {
                (*oap).inclusive = incl(&raw mut (*curwin).w_cursor) == -1;
                (*oap).start = start_pos;
                (*oap).motion_type = kMTCharWise;
                return OK;
            }
            let _ = &mut ncount;
        }

        // extend:
        if lt(start_pos, Visual.start) {
            let mut at_start_sent = true;
            decl(&raw mut pos);
            while lt(pos, (*curwin).w_cursor) {
                let c = gchar_pos(&raw mut pos);
                if !ascii_iswhite(c) {
                    at_start_sent = false;
                    break;
                }
                incl(&raw mut pos);
            }
            if !at_start_sent {
                findsent(BACKWARD, 1);
                if equalpos((*curwin).w_cursor, start_pos) {
                    at_start_sent = true;
                } else {
                    findsent(FORWARD, 1);
                }
            }
            if include {
                count *= 2;
            }
            while {
                let old = count;
                count -= 1;
                old != 0
            } {
                if at_start_sent {
                    find_first_blank(&raw mut (*curwin).w_cursor);
                }
                let c = gchar_cursor();
                if !at_start_sent || (!include && !ascii_iswhite(c)) {
                    findsent(BACKWARD, 1);
                }
                at_start_sent = !at_start_sent;
            }
        } else {
            incl(&raw mut pos);
            let mut at_start_sent = true;
            if !equalpos(pos, (*curwin).w_cursor) {
                at_start_sent = false;
                while lt(pos, (*curwin).w_cursor) {
                    let c = gchar_pos(&raw mut pos);
                    if !ascii_iswhite(c) {
                        at_start_sent = true;
                        break;
                    }
                    incl(&raw mut pos);
                }
                if at_start_sent {
                    findsent(BACKWARD, 1);
                } else {
                    (*curwin).w_cursor = start_pos;
                }
            }

            if include {
                count *= 2;
            }
            findsent_forward(count, at_start_sent);
            if *p_sel == b'e' as c_char {
                (*curwin).w_cursor.col += 1;
            }
        }
        OK
    }
}

/// Find block under the cursor, cursor at end (`current_block`).
pub unsafe fn current_block(
    oap: *mut oparg_T,
    mut count: c_int,
    include: bool,
    what: c_int,
    other: c_int,
) -> c_int {
    unsafe {
        let mut pos: *mut pos_T;
        let mut start_pos = pos_T {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut end_pos: *mut pos_T;
        let mut sol = false;

        let old_pos = (*curwin).w_cursor;
        let mut old_end = (*curwin).w_cursor;
        let mut old_start = old_end;

        if !Visual.active || equalpos(Visual.start, (*curwin).w_cursor) {
            setpcmark();
            if what == b'{' as c_int {
                while inindent(1) {
                    if inc_cursor() != 0 {
                        break;
                    }
                }
            }
            if gchar_cursor() == what {
                (*curwin).w_cursor.col += 1;
            }
        } else if lt(Visual.start, (*curwin).w_cursor) {
            old_start = Visual.start;
            (*curwin).w_cursor = Visual.start;
        } else {
            old_end = Visual.start;
        }

        let save_cpo = p_cpo;
        p_cpo = if !vim_strchr(p_cpo, kCpoMatchbsl).is_null() {
            c"%M".as_ptr().cast_mut()
        } else {
            c"%".as_ptr().cast_mut()
        };
        pos = findmatch(ptr::null_mut(), what);
        if !pos.is_null() {
            while {
                let old = count;
                count -= 1;
                old > 0
            } {
                pos = findmatch(ptr::null_mut(), what);
                if pos.is_null() {
                    break;
                }
                (*curwin).w_cursor = *pos;
                start_pos = *pos;
            }
        } else {
            while {
                let old = count;
                count -= 1;
                old > 0
            } {
                pos = findmatchlimit(ptr::null_mut(), what, FM_FORWARD, 0);
                if pos.is_null() {
                    break;
                }
                (*curwin).w_cursor = *pos;
                start_pos = *pos;
            }
        }
        p_cpo = save_cpo;

        if pos.is_null() || {
            end_pos = findmatch(ptr::null_mut(), other);
            end_pos.is_null()
        } {
            (*curwin).w_cursor = old_pos;
            return FAIL;
        }
        (*curwin).w_cursor = *end_pos;

        if !include {
            loop {
                incl(&raw mut start_pos);
                sol = (*curwin).w_cursor.col == 0;
                decl(&raw mut (*curwin).w_cursor);
                while inindent(1) {
                    sol = true;
                    if decl(&raw mut (*curwin).w_cursor) != 0 {
                        break;
                    }
                }

                if equalpos(start_pos, *end_pos) && Visual.active {
                    (*curwin).w_cursor = old_pos;
                    return FAIL;
                }

                if !lt(start_pos, old_start)
                    && !lt(old_end, (*curwin).w_cursor)
                    && !equalpos(start_pos, (*curwin).w_cursor)
                    && Visual.active
                {
                    (*curwin).w_cursor = old_start;
                    decl(&raw mut (*curwin).w_cursor);
                    pos = findmatch(ptr::null_mut(), what);
                    if pos.is_null() {
                        (*curwin).w_cursor = old_pos;
                        return FAIL;
                    }
                    start_pos = *pos;
                    (*curwin).w_cursor = *pos;
                    end_pos = findmatch(ptr::null_mut(), other);
                    if end_pos.is_null() {
                        (*curwin).w_cursor = old_pos;
                        return FAIL;
                    }
                    (*curwin).w_cursor = *end_pos;
                } else {
                    break;
                }
            }
        }

        if Visual.active {
            if *p_sel == b'e' as c_char {
                inc(&raw mut (*curwin).w_cursor);
            }
            if sol && gchar_cursor() != c_int::from(NUL) {
                inc(&raw mut (*curwin).w_cursor);
            }
            Visual.start = start_pos;
            Visual.mode = b'v' as c_int;
            redraw_curbuf_later(UPD_INVERTED);
            showmode();
        } else {
            (*oap).start = start_pos;
            (*oap).motion_type = kMTCharWise;
            (*oap).inclusive = false;
            if sol {
                incl(&raw mut (*curwin).w_cursor);
            } else if ltoreq(start_pos, (*curwin).w_cursor) {
                (*oap).inclusive = true;
            } else {
                (*curwin).w_cursor = start_pos;
            }
        }

        OK
    }
}

/// Return true if the cursor is on a `<aaa>` tag (or `</aaa>` when `end_tag` is true) (`in_html_tag`).
unsafe fn in_html_tag(end_tag: bool) -> bool {
    unsafe {
        let line = get_cursor_line_ptr();
        let mut p = line.add((*curwin).w_cursor.col as usize);
        let mut lc: c_int = c_int::from(NUL);

        while p > line {
            if *p == b'<' as c_char {
                break;
            }
            let off = utf_head_off(line, p.sub(1)) + 1;
            p = p.sub(off as usize);
            if *p == b'>' as c_char {
                break;
            }
        }
        if *p != b'<' as c_char {
            return false;
        }

        let mut pos = pos_T {
            lnum: (*curwin).w_cursor.lnum,
            col: p.offset_from(line) as colnr_T,
            coladd: 0,
        };

        p = p.add(utfc_ptr2len(p) as usize);
        if end_tag {
            return *p == b'/' as c_char;
        }

        if *p == b'/' as c_char {
            return false;
        }

        loop {
            if inc(&raw mut pos) < 0 {
                return false;
            }
            let c = c_int::from(*ml_get_pos(&raw const pos) as u8);
            if c == b'>' as c_int {
                break;
            }
            lc = c;
        }
        lc != b'/' as c_int
    }
}

/// Find tag block under the cursor, cursor at end (`current_tagblock`).
pub unsafe fn current_tagblock(oap: *mut oparg_T, count_arg: c_int, include: bool) -> c_int {
    unsafe {
        let mut count = count_arg;
        let mut do_include = include;
        let save_p_ws = p_ws;
        let mut retval = FAIL;
        let mut is_inclusive = true;

        p_ws = 0;

        let old_pos = (*curwin).w_cursor;
        let mut old_end = (*curwin).w_cursor;
        let mut old_start = old_end;
        if !Visual.active || *p_sel == b'e' as c_char {
            decl(&raw mut old_end);
        }

        if !Visual.active || equalpos(Visual.start, (*curwin).w_cursor) {
            setpcmark();

            while inindent(1) {
                if inc_cursor() != 0 {
                    break;
                }
            }

            if in_html_tag(false) {
                while *get_cursor_pos_ptr() != b'>' as c_char {
                    if inc_cursor() < 0 {
                        break;
                    }
                }
            } else if in_html_tag(true) {
                while *get_cursor_pos_ptr() != b'<' as c_char {
                    if dec_cursor() < 0 {
                        break;
                    }
                }
                dec_cursor();
                old_end = (*curwin).w_cursor;
            }
        } else if lt(Visual.start, (*curwin).w_cursor) {
            old_start = Visual.start;
            (*curwin).w_cursor = Visual.start;
        } else {
            old_end = Visual.start;
        }

        'again: loop {
            for _ in 0..count {
                if do_searchpair(
                    c"<[^ \t>/!]\\+\\%(\\_s\\_[^>]\\{-}[^/]>\\|$\\|\\_s\\=>\\)".as_ptr(),
                    c"".as_ptr(),
                    c"</[^>]*>".as_ptr(),
                    BACKWARD,
                    ptr::null(),
                    0,
                    ptr::null_mut(),
                    0,
                    0,
                ) <= 0
                {
                    (*curwin).w_cursor = old_pos;
                    p_ws = save_p_ws;
                    return retval;
                }
            }
            let mut start_pos = (*curwin).w_cursor;

            inc_cursor();
            let p = get_cursor_pos_ptr();
            let mut cp = p;
            while *cp != NUL && *cp != b'>' as c_char && !ascii_iswhite(c_int::from(*cp as u8)) {
                cp = cp.add(utfc_ptr2len(cp) as usize);
            }
            let len = cp.offset_from(p) as usize;
            if len == 0 {
                (*curwin).w_cursor = old_pos;
                p_ws = save_p_ws;
                return retval;
            }

            let tag_bytes = std::slice::from_raw_parts(p.cast::<u8>(), len);
            let mut spat = Vec::with_capacity(len + 39);
            spat.extend_from_slice(b"<");
            spat.extend_from_slice(tag_bytes);
            spat.extend_from_slice(b"\\>\\%(\\_s\\_[^>]\\{-}\\_[^/]>\\|\\_s\\?>\\)\\c\0");

            let mut epat = Vec::with_capacity(len + 9);
            epat.extend_from_slice(b"</");
            epat.extend_from_slice(tag_bytes);
            epat.extend_from_slice(b">\\c\0");

            let r = do_searchpair(
                spat.as_ptr().cast::<c_char>(),
                c"".as_ptr(),
                epat.as_ptr().cast::<c_char>(),
                FORWARD,
                ptr::null(),
                0,
                ptr::null_mut(),
                0,
                0,
            );

            if r < 1 || lt((*curwin).w_cursor, old_end) {
                count = 1;
                (*curwin).w_cursor = start_pos;
                continue 'again;
            }

            if do_include {
                while *get_cursor_pos_ptr() != b'>' as c_char {
                    if inc_cursor() < 0 {
                        break;
                    }
                }
            } else {
                let c = get_cursor_pos_ptr();
                if *c == b'<' as c_char && !Visual.active && (*curwin).w_cursor.col == 0 {
                    is_inclusive = false;
                } else if *c == b'<' as c_char {
                    dec_cursor();
                }
            }
            let end_pos = (*curwin).w_cursor;

            if !do_include {
                let mut in_quotes = false;
                (*curwin).w_cursor = start_pos;
                while inc_cursor() >= 0 {
                    let p_cur = get_cursor_pos_ptr();
                    if *p_cur == b'>' as c_char && !in_quotes {
                        inc_cursor();
                        start_pos = (*curwin).w_cursor;
                        break;
                    } else if *p_cur == b'"' as c_char || *p_cur == b'\'' as c_char {
                        in_quotes = !in_quotes;
                    }
                }
                (*curwin).w_cursor = end_pos;

                if Visual.active && equalpos(start_pos, old_start) && equalpos(end_pos, old_end) {
                    do_include = true;
                    (*curwin).w_cursor = old_start;
                    count = count_arg;
                    continue 'again;
                }
            }

            if Visual.active {
                if lt(end_pos, start_pos) {
                    (*curwin).w_cursor = start_pos;
                } else if *p_sel == b'e' as c_char {
                    inc_cursor();
                }
                Visual.start = start_pos;
                Visual.mode = b'v' as c_int;
                redraw_curbuf_later(UPD_INVERTED);
                showmode();
            } else {
                (*oap).start = start_pos;
                (*oap).motion_type = kMTCharWise;
                if lt(end_pos, start_pos) {
                    (*curwin).w_cursor = start_pos;
                    (*oap).inclusive = false;
                } else {
                    (*oap).inclusive = is_inclusive;
                }
            }
            retval = OK;
            break 'again;
        }

        p_ws = save_p_ws;
        retval
    }
}

/// Find paragraph under the cursor (`current_par`).
pub unsafe fn current_par(oap: *mut oparg_T, count: c_int, include: bool, type_: c_int) -> c_int {
    unsafe {
        let mut retval = OK;
        let mut do_white = false;

        if type_ == b'S' as c_int {
            return FAIL;
        }

        let mut start_lnum = (*curwin).w_cursor.lnum;

        let do_extend = Visual.active && start_lnum != Visual.start.lnum;
        if !do_extend {
            let white_in_front = linewhite(start_lnum);
            while start_lnum > 1 {
                if white_in_front {
                    if !linewhite(start_lnum - 1) {
                        break;
                    }
                } else if linewhite(start_lnum - 1) || startPS(start_lnum, 0, false) {
                    break;
                }
                start_lnum -= 1;
            }

            let mut end_lnum = start_lnum;
            while end_lnum <= (*curbuf).b_ml.ml_line_count && linewhite(end_lnum) {
                end_lnum += 1;
            }

            end_lnum -= 1;
            let mut i = count;
            if !include && white_in_front {
                i -= 1;
            }
            while {
                let old = i;
                i -= 1;
                old != 0
            } {
                if end_lnum == (*curbuf).b_ml.ml_line_count {
                    return FAIL;
                }

                if !include {
                    do_white = linewhite(end_lnum + 1);
                }

                if include || !do_white {
                    end_lnum += 1;
                    while end_lnum < (*curbuf).b_ml.ml_line_count
                        && !linewhite(end_lnum + 1)
                        && !startPS(end_lnum + 1, 0, false)
                    {
                        end_lnum += 1;
                    }
                }

                if i == 0 && white_in_front && include {
                    break;
                }

                if include || do_white {
                    while end_lnum < (*curbuf).b_ml.ml_line_count && linewhite(end_lnum + 1) {
                        end_lnum += 1;
                    }
                }
            }

            if !white_in_front && !linewhite(end_lnum) && include {
                while start_lnum > 1 && linewhite(start_lnum - 1) {
                    start_lnum -= 1;
                }
            }

            if Visual.active {
                if !(Visual.mode == b'V' as c_int && start_lnum == (*curwin).w_cursor.lnum) {
                    if Visual.start.lnum != start_lnum {
                        Visual.start.lnum = start_lnum;
                        Visual.start.col = 0;
                    }
                    Visual.mode = b'V' as c_int;
                    redraw_curbuf_later(UPD_INVERTED);
                    showmode();
                    (*curwin).w_cursor.lnum = end_lnum;
                    (*curwin).w_cursor.col = 0;
                    return OK;
                }
            } else {
                (*oap).start.lnum = start_lnum;
                (*oap).start.col = 0;
                (*oap).motion_type = kMTLineWise;
                (*curwin).w_cursor.lnum = end_lnum;
                (*curwin).w_cursor.col = 0;
                return OK;
            }
        }

        // extend:
        let dir = if start_lnum < Visual.start.lnum {
            BACKWARD
        } else {
            FORWARD
        };
        let mut i = count;
        while {
            i -= 1;
            i >= 0
        } {
            if start_lnum
                == (if dir == BACKWARD {
                    1
                } else {
                    (*curbuf).b_ml.ml_line_count
                })
            {
                retval = FAIL;
                break;
            }

            let mut prev_start_is_white: c_int = -1;
            for _ in 0..2 {
                start_lnum += dir;
                let start_is_white = linewhite(start_lnum);
                if prev_start_is_white == c_int::from(start_is_white) {
                    start_lnum -= dir;
                    break;
                }
                loop {
                    if start_lnum
                        == (if dir == BACKWARD {
                            1
                        } else {
                            (*curbuf).b_ml.ml_line_count
                        })
                    {
                        break;
                    }
                    if start_is_white != linewhite(start_lnum + dir)
                        || (!start_is_white
                            && startPS(start_lnum + if dir > 0 { 1 } else { 0 }, 0, false))
                    {
                        break;
                    }
                    start_lnum += dir;
                }
                if !include {
                    break;
                }
                if start_lnum
                    == (if dir == BACKWARD {
                        1
                    } else {
                        (*curbuf).b_ml.ml_line_count
                    })
                {
                    break;
                }
                prev_start_is_white = c_int::from(start_is_white);
            }
        }
        (*curwin).w_cursor.lnum = start_lnum;
        (*curwin).w_cursor.col = 0;
        retval
    }
}

/// Search quote char from string `line[col]` (`find_next_quote`).
unsafe fn find_next_quote(
    line: *mut c_char,
    mut col: c_int,
    quotechar: c_int,
    escape: *mut c_char,
) -> c_int {
    unsafe {
        loop {
            let c = c_int::from(*line.add(col as usize) as u8);
            if c == c_int::from(NUL) {
                return -1;
            } else if !escape.is_null() && !vim_strchr(escape, c).is_null() {
                col += 1;
                if *line.add(col as usize) == NUL {
                    return -1;
                }
            } else if c == quotechar {
                break;
            }
            col += utfc_ptr2len(line.add(col as usize));
        }
        col
    }
}

/// Search backwards in `line` from column `col_start` to find `quotechar` (`find_prev_quote`).
unsafe fn find_prev_quote(
    line: *mut c_char,
    mut col_start: c_int,
    quotechar: c_int,
    escape: *mut c_char,
) -> c_int {
    unsafe {
        while col_start > 0 {
            col_start -= 1;
            col_start -= utf_head_off(line, line.add(col_start as usize));
            let mut n = 0;
            if !escape.is_null() {
                while col_start - n > 0
                    && !vim_strchr(
                        escape,
                        c_int::from(*line.add((col_start - n - 1) as usize) as u8),
                    )
                    .is_null()
                {
                    n += 1;
                }
            }
            if (n & 1) != 0 {
                col_start -= n;
            } else if c_int::from(*line.add(col_start as usize) as u8) == quotechar {
                break;
            }
        }
        col_start
    }
}

/// Find quote under the cursor, cursor at end (`current_quote`).
pub unsafe fn current_quote(
    oap: *mut oparg_T,
    count: c_int,
    include: bool,
    quotechar: c_int,
) -> bool {
    unsafe {
        let line = get_cursor_line_ptr();
        let mut col_end: c_int;
        let mut col_start: c_int = (*curwin).w_cursor.col;
        let mut inclusive = false;
        let mut vis_empty = true;
        let mut vis_bef_curs = false;
        let mut did_exclusive_adj = false;
        let mut inside_quotes = false;
        let mut selected_quote = false;
        let mut restore_vis_bef = false;

        let abort_search = |did_exclusive_adj: bool, restore_vis_bef: bool| -> bool {
            if Visual.active && *p_sel == b'e' as c_char {
                if did_exclusive_adj {
                    inc_cursor();
                }
                if restore_vis_bef {
                    ptr::swap(&raw mut (*curwin).w_cursor, &raw mut Visual.start);
                }
            }
            false
        };

        if Visual.active {
            if Visual.start.lnum != (*curwin).w_cursor.lnum {
                return false;
            }

            vis_bef_curs = lt(Visual.start, (*curwin).w_cursor);
            vis_empty = equalpos(Visual.start, (*curwin).w_cursor);
            if *p_sel == b'e' as c_char {
                if vis_bef_curs {
                    dec_cursor();
                    did_exclusive_adj = true;
                } else if !vis_empty {
                    dec(&raw mut Visual.start);
                    did_exclusive_adj = true;
                }
                vis_empty = equalpos(Visual.start, (*curwin).w_cursor);
                if !vis_bef_curs && !vis_empty {
                    ptr::swap(&raw mut (*curwin).w_cursor, &raw mut Visual.start);
                    vis_bef_curs = true;
                    restore_vis_bef = true;
                }
            }
        }

        if !vis_empty {
            let mut i: c_int;
            if vis_bef_curs {
                inside_quotes = Visual.start.col > 0
                    && c_int::from(*line.add((Visual.start.col - 1) as usize) as u8) == quotechar
                    && *line.add((*curwin).w_cursor.col as usize) != NUL
                    && c_int::from(*line.add(((*curwin).w_cursor.col + 1) as usize) as u8)
                        == quotechar;
                i = Visual.start.col;
                col_end = (*curwin).w_cursor.col;
            } else {
                inside_quotes = (*curwin).w_cursor.col > 0
                    && c_int::from(*line.add(((*curwin).w_cursor.col - 1) as usize) as u8)
                        == quotechar
                    && *line.add(Visual.start.col as usize) != NUL
                    && c_int::from(*line.add((Visual.start.col + 1) as usize) as u8) == quotechar;
                i = (*curwin).w_cursor.col;
                col_end = Visual.start.col;
            }

            while i <= col_end {
                if *line.add(i as usize) == NUL {
                    break;
                }
                let ch = c_int::from(*line.add(i as usize) as u8);
                i += 1;
                if ch == quotechar {
                    selected_quote = true;
                    break;
                }
            }
        }

        if !vis_empty && c_int::from(*line.add(col_start as usize) as u8) == quotechar {
            if vis_bef_curs {
                col_start = find_next_quote(line, col_start + 1, quotechar, ptr::null_mut());
                if col_start < 0 {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                col_end = find_next_quote(line, col_start + 1, quotechar, (*curbuf).b_p_qe);
                if col_end < 0 {
                    col_end = col_start;
                    col_start = (*curwin).w_cursor.col;
                }
            } else {
                col_end = find_prev_quote(line, col_start, quotechar, ptr::null_mut());
                if c_int::from(*line.add(col_end as usize) as u8) != quotechar {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                col_start = find_prev_quote(line, col_end, quotechar, (*curbuf).b_p_qe);
                if c_int::from(*line.add(col_start as usize) as u8) != quotechar {
                    col_start = col_end;
                    col_end = (*curwin).w_cursor.col;
                }
            }
        } else if c_int::from(*line.add(col_start as usize) as u8) == quotechar || !vis_empty {
            let mut first_col = col_start;

            if !vis_empty {
                if vis_bef_curs {
                    first_col = find_next_quote(line, col_start, quotechar, ptr::null_mut());
                } else {
                    first_col = find_prev_quote(line, col_start, quotechar, ptr::null_mut());
                }
            }
            col_start = 0;
            loop {
                col_start = find_next_quote(line, col_start, quotechar, ptr::null_mut());
                if col_start < 0 || col_start > first_col {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                col_end = find_next_quote(line, col_start + 1, quotechar, (*curbuf).b_p_qe);
                if col_end < 0 {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                if col_start <= first_col && first_col <= col_end {
                    break;
                }
                col_start = col_end + 1;
            }
        } else {
            col_start = find_prev_quote(line, col_start, quotechar, (*curbuf).b_p_qe);
            if c_int::from(*line.add(col_start as usize) as u8) != quotechar {
                col_start = find_next_quote(line, col_start, quotechar, ptr::null_mut());
                if col_start < 0 {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
            }

            col_end = find_next_quote(line, col_start + 1, quotechar, (*curbuf).b_p_qe);
            if col_end < 0 {
                return abort_search(did_exclusive_adj, restore_vis_bef);
            }
        }

        if include {
            if ascii_iswhite(c_int::from(*line.add((col_end + 1) as usize) as u8)) {
                while ascii_iswhite(c_int::from(*line.add((col_end + 1) as usize) as u8)) {
                    col_end += 1;
                }
            } else {
                while col_start > 0
                    && ascii_iswhite(c_int::from(*line.add((col_start - 1) as usize) as u8))
                {
                    col_start -= 1;
                }
            }
        }

        if !include && count < 2 && (vis_empty || !inside_quotes) {
            col_start += 1;
        }
        (*curwin).w_cursor.col = col_start;
        if Visual.active {
            if vis_empty
                || (vis_bef_curs
                    && !selected_quote
                    && (inside_quotes
                        || (c_int::from(*line.add(Visual.start.col as usize) as u8) != quotechar
                            && (Visual.start.col == 0
                                || c_int::from(*line.add((Visual.start.col - 1) as usize) as u8)
                                    != quotechar))))
            {
                Visual.start = (*curwin).w_cursor;
                redraw_curbuf_later(UPD_INVERTED);
            }
        } else {
            (*oap).start = (*curwin).w_cursor;
            (*oap).motion_type = kMTCharWise;
        }

        (*curwin).w_cursor.col = col_end;
        if (include || count > 1 || (!vis_empty && inside_quotes)) && inc_cursor() == 2 {
            inclusive = true;
        }
        if Visual.active {
            if vis_empty || vis_bef_curs {
                if *p_sel != b'e' as c_char {
                    dec_cursor();
                }
            } else {
                if inside_quotes
                    || (!selected_quote
                        && c_int::from(*line.add(Visual.start.col as usize) as u8) != quotechar
                        && (*line.add(Visual.start.col as usize) == NUL
                            || c_int::from(*line.add((Visual.start.col + 1) as usize) as u8)
                                != quotechar))
                {
                    dec_cursor();
                    Visual.start = (*curwin).w_cursor;
                }
                (*curwin).w_cursor.col = col_start;
            }
            if Visual.mode == b'V' as c_int {
                Visual.mode = b'v' as c_int;
                redraw_cmdline = true;
            }
        } else {
            (*oap).inclusive = inclusive;
        }

        true
    }
}
