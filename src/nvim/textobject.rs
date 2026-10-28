//! Functions for text objects (`src/nvim/textobject.c`).

use crate::charset::{NUL, ascii_iswhite, skipwhite};
use crate::cursor::{
    Colnr, CursorPos, Linenr, clearpos, coladvance, dec_cursor, equalpos, gchar_cursor,
    get_cursor_line_ptr, get_cursor_pos_ptr, inc_cursor, lt, ltoreq,
};
use crate::drawscreen::{UPD_INVERTED, redraw_curbuf_later, showmode};
use crate::eval::funcs::{MT_CHAR_WISE, MT_LINE_WISE, do_searchpair};
use crate::fold::has_folding;
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
    BACKWARD, Direction, FORWARD, K_CPO_MATCHBSL, findmatch, findmatchlimit, linewhite,
};
use crate::strings::vim_strchr;
use crate::textformat::Oparg;
use std::ffi::c_char;
use std::ptr;

use crate::regexp::MAXCOL;

const K_CPO_ENDOFSENT: i32 = b'J' as i32;

struct TextobjectState {
    cls_bigword: bool,
}

impl TextobjectState {
    const fn new() -> Self {
        Self { cls_bigword: false }
    }
}

static TOBJ_STATE: crate::globals::GlobalState<TextobjectState> =
    crate::globals::GlobalState::new(TextobjectState::new());

#[inline]
fn tobj_st() -> &'static mut TextobjectState {
    unsafe { &mut *TOBJ_STATE.as_ptr() }
}

/// Find the start of the next sentence, searching in the direction specified
/// by the `dir` argument (`findsent`).
pub fn findsent(dir: Direction, mut count: i32) -> bool {
    {
        let mut c: i32;
        let func: fn(&mut CursorPos) -> i32 = if dir == FORWARD { incl } else { decl };
        let mut noskip = false;

        let mut pos = curwin_mut().w_cursor;

        while {
            let old = count;
            count -= 1;
            old != 0
        } {
            let prev_pos = pos;
            let mut goto_found = false;

            // if on an empty line, skip up to a non-empty line
            if gchar_pos(&mut pos) == i32::from(NUL) {
                loop {
                    if func(&mut pos) == -1 {
                        break;
                    }
                    if gchar_pos(&mut pos) != i32::from(NUL) {
                        break;
                    }
                }
                if dir == FORWARD {
                    goto_found = true;
                }
            } else if dir == FORWARD && pos.col == 0 && start_ps(pos.lnum, i32::from(NUL), false) {
                // if on the start of a paragraph or a section and searching forward,
                // go to the next line
                if pos.lnum == curbuf_mut().b_ml.ml_line_count {
                    return false;
                }
                pos.lnum += 1;
                goto_found = true;
            } else if dir == BACKWARD {
                decl(&mut pos);
            }

            if !goto_found {
                // go back to the previous non-white non-punctuation character
                let mut found_dot = false;
                while {
                    c = gchar_pos(&mut pos);
                    ascii_iswhite(c) || !vim_strchr(c".!?)]\"'", c).is_null()
                } {
                    let mut tpos = pos;
                    if decl(&mut tpos) == -1 || (lineempty(tpos.lnum) && dir == FORWARD) {
                        break;
                    }
                    if found_dot {
                        break;
                    }
                    if crate::strings::vim_haschr(c".!?", c) {
                        found_dot = true;
                    }
                    if !vim_strchr(c")]\"'", c).is_null()
                        && vim_strchr(c".!?)]\"'", gchar_pos(&mut tpos)).is_null()
                    {
                        break;
                    }
                    decl(&mut pos);
                }

                // remember the line where the search started
                let startlnum = pos.lnum;
                let cpo_j = crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_ENDOFSENT);

                loop {
                    c = gchar_pos(&mut pos);
                    if c == i32::from(NUL)
                        || (pos.col == 0 && start_ps(pos.lnum, i32::from(NUL), false))
                    {
                        if dir == BACKWARD && pos.lnum != startlnum {
                            pos.lnum += 1;
                        }
                        break;
                    }
                    if c == b'.' as i32 || c == b'!' as i32 || c == b'?' as i32 {
                        let mut tpos = pos;
                        loop {
                            c = inc(&mut tpos);
                            if c == -1 {
                                break;
                            }
                            c = gchar_pos(&mut tpos);
                            if vim_strchr(c")]\"'", c).is_null() {
                                break;
                            }
                        }
                        if c == -1
                            || (!cpo_j && (c == b' ' as i32 || c == b'\t' as i32))
                            || c == i32::from(NUL)
                            || (cpo_j
                                && c == b' ' as i32
                                && inc(&mut tpos) >= 0
                                && gchar_pos(&mut tpos) == b' ' as i32)
                        {
                            pos = tpos;
                            if gchar_pos(&mut pos) == i32::from(NUL) {
                                inc(&mut pos);
                            }
                            break;
                        }
                    }
                    if func(&mut pos) == -1 {
                        if count != 0 {
                            return false;
                        }
                        noskip = true;
                        break;
                    }
                }
            }

            // skip white space
            while !noskip && {
                c = gchar_pos(&mut pos);
                c == b' ' as i32 || c == b'\t' as i32
            } {
                if incl(&mut pos) == -1 {
                    break;
                }
            }

            if equalpos(prev_pos, pos) {
                // didn't actually move, advance one character and try again
                if func(&mut pos) == -1 {
                    if count != 0 {
                        return false;
                    }
                    break;
                }
                count += 1;
            }
        }

        setpcmark();
        curwin_mut().w_cursor = pos;
        true
    }
}

/// Find the next paragraph or section in direction `dir` (`findpar`).
pub fn findpar(pincl: &mut bool, dir: i32, mut count: i32, what: i32, both: bool) -> bool {
    unsafe {
        let mut fold_first: Linenr = 0;
        let mut fold_last: Linenr = 0;
        let mut curr = curwin_mut().w_cursor.lnum;

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
                if first
                    && has_folding(
                        curwin_ptr(),
                        curr,
                        Some(&mut fold_first),
                        &raw mut fold_last,
                    )
                {
                    curr = (if dir > 0 { fold_last } else { fold_first }) + dir;
                    fold_skipped = true;
                }

                if !first && did_skip && start_ps(curr, what, both) {
                    break;
                }

                if fold_skipped {
                    curr -= dir;
                }
                curr += dir;
                if curr < 1 || curr > curbuf_mut().b_ml.ml_line_count {
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
        curwin_mut().w_cursor.lnum = curr;
        if curr == curbuf_mut().b_ml.ml_line_count && what != b'}' as i32 && dir == FORWARD {
            let line = ml_get(curr);

            // Put the cursor on the last character in the last line and make the
            // motion inclusive.
            curwin_mut().w_cursor.col = ml_get_len(curr);
            if curwin_mut().w_cursor.col != 0 {
                curwin_mut().w_cursor.col -= 1;
                curwin_mut().w_cursor.col -=
                    utf_head_off(line, line.wrapping_add(curwin_mut().w_cursor.col as usize));
                *pincl = true;
            }
        } else {
            curwin_mut().w_cursor.col = 0;
        }
        true
    }
}

/// Check if the string `s` is a nroff macro that is in option `opt` (`inmacro`).
fn inmacro(opt: impl crate::strings::IntoCCharPtr, s: impl crate::strings::IntoCCharPtr) -> bool {
    let opt = opt.into_c_char_ptr();
    let s = s.into_c_char_ptr();
    unsafe {
        let mut macro_p = opt;
        let s0 = *s;
        let s1 = if s0 == NUL { NUL } else { *s.wrapping_add(1) };

        while !crate::memory::cstr_is_empty(macro_p) {
            let m0 = *macro_p;
            let m1 = *macro_p.wrapping_add(1);
            if (m0 == s0 || (m0 == b' ' as c_char && (s0 == NUL || s0 == b' ' as c_char)))
                && (m1 == s1
                    || ((m1 == NUL || m1 == b' ' as c_char)
                        && (s0 == NUL || s1 == NUL || s1 == b' ' as c_char)))
            {
                break;
            }
            macro_p = macro_p.wrapping_add(1);
            if crate::memory::cstr_is_empty(macro_p) {
                break;
            }
            macro_p = macro_p.wrapping_add(1);
        }
        !crate::memory::cstr_is_empty(macro_p)
    }
}

/// Return true if line `lnum` is the start of a section or paragraph (`startPS`).
pub fn start_ps(lnum: Linenr, para: i32, both: bool) -> bool {
    {
        let s = ml_get(lnum);
        if i32::from(crate::memory::cstr_first_byte(s)) == para
            || crate::memory::cstr_first_byte(s) == b'\x0c'
            || (both && crate::memory::cstr_first_byte(s) == b'}')
        {
            return true;
        }
        if crate::memory::cstr_first_byte(s) == b'.'
            && (inmacro(opt_vars().p_sections, s.wrapping_add(1))
                || (para == 0 && inmacro(opt_vars().p_para, s.wrapping_add(1))))
        {
            return true;
        }
        false
    }
}

/// Return the class of character at `curwin->w_cursor` (`cls`).
fn cls() -> i32 {
    let mut c = gchar_cursor();
    if c == b' ' as i32 || c == b'\t' as i32 || c == i32::from(NUL) {
        return 0;
    }

    c = utf_class(c);

    if c != 0 && tobj_st().cls_bigword {
        return 1;
    }
    c
}

/// Move forward `count` words (`fwd_word`).
pub fn fwd_word(mut count: i32, bigword: bool, eol: bool) -> bool {
    {
        curwin_mut().w_cursor.coladd = 0;
        tobj_st().cls_bigword = bigword;
        while {
            count -= 1;
            count >= 0
        } {
            if has_folding(
                curwin_ptr(),
                curwin_mut().w_cursor.lnum,
                None,
                &raw mut curwin_mut().w_cursor.lnum,
            ) {
                coladvance(curwin_mut(), MAXCOL);
            }
            let sclass = cls();

            let last_line = curwin_mut().w_cursor.lnum == curbuf_mut().b_ml.ml_line_count;
            let mut i = inc_cursor();
            if i == -1 || (i >= 1 && last_line) {
                return false;
            }
            if i >= 1 && eol && count == 0 {
                return true;
            }

            if sclass != 0 {
                while cls() == sclass {
                    i = inc_cursor();
                    if i == -1 || (i >= 1 && eol && count == 0) {
                        return true;
                    }
                }
            }

            while cls() == 0 {
                if curwin_mut().w_cursor.col == 0
                    && crate::memory::cstr_is_empty(get_cursor_line_ptr())
                {
                    break;
                }

                i = inc_cursor();
                if i == -1 || (i >= 1 && eol && count == 0) {
                    return true;
                }
            }
        }
        true
    }
}

/// Move backward `count` words (`bck_word`).
pub fn bck_word(mut count: i32, bigword: bool, mut stop: bool) -> bool {
    {
        curwin_mut().w_cursor.coladd = 0;
        tobj_st().cls_bigword = bigword;
        while {
            count -= 1;
            count >= 0
        } {
            if has_folding(
                curwin_ptr(),
                curwin_mut().w_cursor.lnum,
                Some(&mut curwin_mut().w_cursor.lnum),
                ptr::null_mut(),
            ) {
                curwin_mut().w_cursor.col = 0;
            }
            let sclass = cls();
            if dec_cursor() == -1 {
                return false;
            }

            let mut finished = false;
            if !stop || sclass == cls() || sclass == 0 {
                while cls() == 0 {
                    if curwin_mut().w_cursor.col == 0 && lineempty(curwin_mut().w_cursor.lnum) {
                        finished = true;
                        break;
                    }
                    if dec_cursor() == -1 {
                        return true;
                    }
                }

                if !finished && skip_chars(cls(), BACKWARD) {
                    return true;
                }
            }

            if !finished {
                inc_cursor();
            }
            stop = false;
        }
        adjust_skipcol();
        true
    }
}

/// Move to the end of the word (`end_word`).
pub fn end_word(mut count: i32, bigword: bool, mut stop: bool, empty: bool) -> bool {
    {
        curwin_mut().w_cursor.coladd = 0;
        tobj_st().cls_bigword = bigword;

        if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            && g_st().visual_global.active
            && g_st().visual_global.mode == b'v' as i32
            && g_st().visual_global.select_exclu_adj
        {
            unadjust_for_sel();
        }

        while {
            count -= 1;
            count >= 0
        } {
            if has_folding(
                curwin_ptr(),
                curwin_mut().w_cursor.lnum,
                None,
                &raw mut curwin_mut().w_cursor.lnum,
            ) {
                coladvance(curwin_mut(), MAXCOL);
            }
            let sclass = cls();
            if inc_cursor() == -1 {
                return false;
            }

            let mut finished = false;
            if cls() == sclass && sclass != 0 {
                if skip_chars(sclass, FORWARD) {
                    return false;
                }
            } else if !stop || sclass == 0 {
                while cls() == 0 {
                    if empty
                        && curwin_mut().w_cursor.col == 0
                        && lineempty(curwin_mut().w_cursor.lnum)
                    {
                        finished = true;
                        break;
                    }
                    if inc_cursor() == -1 {
                        return false;
                    }
                }

                if !finished && skip_chars(cls(), FORWARD) {
                    return false;
                }
            }
            if !finished {
                dec_cursor();
            }
            stop = false;
        }
        true
    }
}

/// Move back to the end of the word (`bckend_word`).
pub fn bckend_word(mut count: i32, bigword: bool, eol: bool) -> bool {
    {
        curwin_mut().w_cursor.coladd = 0;
        tobj_st().cls_bigword = bigword;
        while {
            count -= 1;
            count >= 0
        } {
            let sclass = cls();
            let mut i = dec_cursor();
            if i == -1 {
                return false;
            }
            if eol && i == 1 {
                return true;
            }

            if sclass != 0 {
                while cls() == sclass {
                    i = dec_cursor();
                    if i == -1 || (eol && i == 1) {
                        return true;
                    }
                }
            }

            while cls() == 0 {
                if curwin_mut().w_cursor.col == 0 && lineempty(curwin_mut().w_cursor.lnum) {
                    break;
                }
                i = dec_cursor();
                if i == -1 || (eol && i == 1) {
                    return true;
                }
            }
        }
        adjust_skipcol();
        true
    }
}

/// Skip a row of characters of the same class (`skip_chars`).
fn skip_chars(cclass: i32, dir: Direction) -> bool {
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

/// Go back to the start of the word or the start of white space (`back_in_line`).
fn back_in_line() {
    {
        let sclass = cls();
        loop {
            if curwin_mut().w_cursor.col == 0 {
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

fn find_first_blank(posp: &mut CursorPos) {
    while decl(posp) != -1 {
        let c = gchar_pos(posp);
        if !ascii_iswhite(c) {
            incl(posp);
            break;
        }
    }
}

/// Skip `count/2` sentences and `count/2` separating white spaces (`findsent_forward`).
fn findsent_forward(mut count: i32, mut at_start_sent: bool) {
    {
        while {
            let old = count;
            count -= 1;
            old != 0
        } {
            findsent(FORWARD, 1);
            if at_start_sent {
                find_first_blank(&mut curwin_mut().w_cursor);
            }
            if count == 0 || at_start_sent {
                decl(&mut curwin_mut().w_cursor);
            }
            at_start_sent = !at_start_sent;
        }
    }
}

/// Find word under cursor, cursor at end (`current_word`).
pub fn current_word(oap: &mut Oparg, mut count: i32, include: bool, bigword: bool) -> bool {
    {
        let mut start_pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut inclusive = true;
        let mut include_white = false;

        tobj_st().cls_bigword = bigword;
        clearpos(&mut start_pos);

        if g_st().visual_global.active
            && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            && lt(g_st().visual_global.start, curwin_mut().w_cursor)
        {
            dec_cursor();
        }

        if !g_st().visual_global.active
            || equalpos(curwin_mut().w_cursor, g_st().visual_global.start)
        {
            back_in_line();
            start_pos = curwin_mut().w_cursor;

            if (cls() == 0) == include {
                if !end_word(1, bigword, true, true) {
                    return false;
                }
            } else {
                fwd_word(1, bigword, true);
                if curwin_mut().w_cursor.col == 0 {
                    decl(&mut curwin_mut().w_cursor);
                } else {
                    oneleft();
                }

                if include {
                    include_white = true;
                }
            }

            if g_st().visual_global.active {
                g_st().visual_global.start = start_pos;
                redraw_curbuf_later(UPD_INVERTED);
            } else {
                oap.start = start_pos;
                oap.motion_type = MT_CHAR_WISE;
            }
            count -= 1;
        }

        while count > 0 {
            inclusive = true;
            if g_st().visual_global.active && lt(curwin_mut().w_cursor, g_st().visual_global.start)
            {
                if decl(&mut curwin_mut().w_cursor) == -1 {
                    return false;
                }
                if include != (cls() != 0) {
                    if !bck_word(1, bigword, true) {
                        return false;
                    }
                } else {
                    if !bckend_word(1, bigword, true) {
                        return false;
                    }
                    let _ = incl(&mut curwin_mut().w_cursor);
                }
            } else {
                if incl(&mut curwin_mut().w_cursor) == -1 {
                    return false;
                }
                if include != (cls() == 0) {
                    if !fwd_word(1, bigword, true) && count > 1 {
                        return false;
                    }
                    if !oneleft() {
                        inclusive = false;
                    }
                } else if !end_word(1, bigword, true, true) {
                    return false;
                }
            }
            count -= 1;
        }

        if include_white && (cls() != 0 || (curwin_mut().w_cursor.col == 0 && !inclusive)) {
            let pos = curwin_mut().w_cursor;
            curwin_mut().w_cursor = start_pos;
            if oneleft() {
                back_in_line();
                if cls() == 0 && curwin_mut().w_cursor.col > 0 {
                    if g_st().visual_global.active {
                        g_st().visual_global.start = curwin_mut().w_cursor;
                    } else {
                        oap.start = curwin_mut().w_cursor;
                    }
                }
            }
            curwin_mut().w_cursor = pos;
        }

        if g_st().visual_global.active {
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
                && inclusive
                && ltoreq(g_st().visual_global.start, curwin_mut().w_cursor)
            {
                inc_cursor();
            }
            if g_st().visual_global.mode == b'V' as i32 {
                g_st().visual_global.mode = b'v' as i32;
                g_st().redraw_cmdline = true;
            }
        } else {
            oap.inclusive = inclusive;
        }

        true
    }
}

/// Find the current line ("il") or all lines ("al") (`current_line`).
pub fn current_line(oap: &mut Oparg, include: bool) -> bool {
    unsafe {
        if include {
            if g_st().visual_global.active {
                g_st().visual_global.start.lnum = 1;
                g_st().visual_global.start.col = 0;
                g_st().visual_global.start.coladd = 0;
                g_st().visual_global.mode = b'V' as i32;
                redraw_curbuf_later(UPD_INVERTED);
                showmode();
            } else {
                oap.cursor_start = curwin_mut().w_cursor;
                oap.restore_cursor = true;
                oap.start.lnum = 1;
                oap.start.col = 0;
                oap.start.coladd = 0;
                oap.motion_type = MT_LINE_WISE;
            }
            curwin_mut().w_cursor.lnum = curbuf_mut().b_ml.ml_line_count;
            curwin_mut().w_cursor.col = 0;
            curwin_mut().w_cursor.coladd = 0;
            return true;
        }

        let line = ml_get(curwin_mut().w_cursor.lnum);
        let start = skipwhite(line);
        let mut end = line.wrapping_add(crate::memory::cstr_len(line));
        while end > start {
            let prev = mb_prevptr(line, end);
            if !ascii_iswhite(i32::from(crate::memory::cstr_first_byte(prev))) {
                break;
            }
            end = prev;
        }
        if start == end {
            return false;
        }

        let mut start_pos = curwin_mut().w_cursor;
        start_pos.col = start.offset_from(line) as Colnr;
        start_pos.coladd = 0;

        let mut end_pos = curwin_mut().w_cursor;
        end_pos.col = mb_prevptr(line, end).offset_from(line) as Colnr;
        end_pos.coladd = 0;

        if g_st().visual_global.active {
            g_st().visual_global.start = start_pos;
            curwin_mut().w_cursor = end_pos;
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
                && ltoreq(g_st().visual_global.start, curwin_mut().w_cursor)
            {
                inc_cursor();
            }
            g_st().visual_global.mode = b'v' as i32;
            redraw_curbuf_later(UPD_INVERTED);
            showmode();
        } else {
            oap.start = start_pos;
            oap.motion_type = MT_CHAR_WISE;
            oap.inclusive = true;
            curwin_mut().w_cursor = end_pos;
        }

        true
    }
}

/// Find sentence(s) under the cursor, cursor at end (`current_sent`).
pub fn current_sent(oap: &mut Oparg, mut count: i32, include: bool) -> bool {
    {
        let mut start_pos = curwin_mut().w_cursor;
        let mut pos = start_pos;
        findsent(FORWARD, 1);

        let do_extend =
            g_st().visual_global.active && !equalpos(start_pos, g_st().visual_global.start);
        if !do_extend {
            while ascii_iswhite(gchar_pos(&mut pos)) {
                incl(&mut pos);
            }
            let start_blank = if equalpos(pos, curwin_mut().w_cursor) {
                find_first_blank(&mut start_pos);
                true
            } else {
                findsent(BACKWARD, 1);
                start_pos = curwin_mut().w_cursor;
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
                decl(&mut curwin_mut().w_cursor);
            }

            if include {
                if start_blank {
                    find_first_blank(&mut curwin_mut().w_cursor);
                    let c = gchar_pos(&mut curwin_mut().w_cursor);
                    if ascii_iswhite(c) {
                        decl(&mut curwin_mut().w_cursor);
                    }
                } else if !ascii_iswhite(gchar_cursor()) {
                    find_first_blank(&mut start_pos);
                }
            }

            if g_st().visual_global.active {
                if !equalpos(start_pos, curwin_mut().w_cursor) {
                    if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                        curwin_mut().w_cursor.col += 1;
                    }
                    g_st().visual_global.start = start_pos;
                    g_st().visual_global.mode = b'v' as i32;
                    g_st().redraw_cmdline = true;
                    redraw_curbuf_later(UPD_INVERTED);
                    return true;
                }
            } else {
                oap.inclusive = incl(&mut curwin_mut().w_cursor) == -1;
                oap.start = start_pos;
                oap.motion_type = MT_CHAR_WISE;
                return true;
            }
            let _ = &mut ncount;
        }

        // extend:
        if lt(start_pos, g_st().visual_global.start) {
            let mut at_start_sent = true;
            decl(&mut pos);
            while lt(pos, curwin_mut().w_cursor) {
                let c = gchar_pos(&mut pos);
                if !ascii_iswhite(c) {
                    at_start_sent = false;
                    break;
                }
                incl(&mut pos);
            }
            if !at_start_sent {
                findsent(BACKWARD, 1);
                if equalpos(curwin_mut().w_cursor, start_pos) {
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
                    find_first_blank(&mut curwin_mut().w_cursor);
                }
                let c = gchar_cursor();
                if !at_start_sent || (!include && !ascii_iswhite(c)) {
                    findsent(BACKWARD, 1);
                }
                at_start_sent = !at_start_sent;
            }
        } else {
            incl(&mut pos);
            let mut at_start_sent = true;
            if !equalpos(pos, curwin_mut().w_cursor) {
                at_start_sent = false;
                while lt(pos, curwin_mut().w_cursor) {
                    let c = gchar_pos(&mut pos);
                    if !ascii_iswhite(c) {
                        at_start_sent = true;
                        break;
                    }
                    incl(&mut pos);
                }
                if at_start_sent {
                    findsent(BACKWARD, 1);
                } else {
                    curwin_mut().w_cursor = start_pos;
                }
            }

            if include {
                count *= 2;
            }
            findsent_forward(count, at_start_sent);
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                curwin_mut().w_cursor.col += 1;
            }
        }
        true
    }
}

/// Find block under the cursor, cursor at end (`current_block`).
pub fn current_block(
    oap: &mut Oparg,
    mut count: i32,
    include: bool,
    what: i32,
    other: i32,
) -> bool {
    unsafe {
        let mut pos: *mut CursorPos;
        let mut start_pos = CursorPos {
            lnum: 0,
            col: 0,
            coladd: 0,
        };
        let mut end_pos: *mut CursorPos;
        let mut sol = false;

        let old_pos = curwin_mut().w_cursor;
        let mut old_end = curwin_mut().w_cursor;
        let mut old_start = old_end;

        if !g_st().visual_global.active
            || equalpos(g_st().visual_global.start, curwin_mut().w_cursor)
        {
            setpcmark();
            if what == b'{' as i32 {
                while inindent(1) {
                    if inc_cursor() != 0 {
                        break;
                    }
                }
            }
            if gchar_cursor() == what {
                curwin_mut().w_cursor.col += 1;
            }
        } else if lt(g_st().visual_global.start, curwin_mut().w_cursor) {
            old_start = g_st().visual_global.start;
            curwin_mut().w_cursor = g_st().visual_global.start;
        } else {
            old_end = g_st().visual_global.start;
        }

        let save_cpo = opt_vars().p_cpo;
        opt_vars().p_cpo = if crate::strings::vim_haschr(opt_vars().p_cpo, K_CPO_MATCHBSL) {
            c"%M".as_ptr().cast_mut()
        } else {
            c"%".as_ptr().cast_mut()
        };
        pos = findmatch(None, what);
        if !pos.is_null() {
            while {
                let old = count;
                count -= 1;
                old > 0
            } {
                pos = findmatch(None, what);
                if pos.is_null() {
                    break;
                }
                curwin_mut().w_cursor = *pos;
                start_pos = *pos;
            }
        } else {
            while {
                let old = count;
                count -= 1;
                old > 0
            } {
                pos = findmatchlimit(None, what, FM_FORWARD, 0);
                if pos.is_null() {
                    break;
                }
                curwin_mut().w_cursor = *pos;
                start_pos = *pos;
            }
        }
        opt_vars().p_cpo = save_cpo;

        if pos.is_null() || {
            end_pos = findmatch(None, other);
            end_pos.is_null()
        } {
            curwin_mut().w_cursor = old_pos;
            return false;
        }
        curwin_mut().w_cursor = *end_pos;

        if !include {
            loop {
                incl(&mut start_pos);
                sol = curwin_mut().w_cursor.col == 0;
                decl(&mut curwin_mut().w_cursor);
                while inindent(1) {
                    sol = true;
                    if decl(&mut curwin_mut().w_cursor) != 0 {
                        break;
                    }
                }

                if equalpos(start_pos, *end_pos) && g_st().visual_global.active {
                    curwin_mut().w_cursor = old_pos;
                    return false;
                }

                if !lt(start_pos, old_start)
                    && !lt(old_end, curwin_mut().w_cursor)
                    && !equalpos(start_pos, curwin_mut().w_cursor)
                    && g_st().visual_global.active
                {
                    curwin_mut().w_cursor = old_start;
                    decl(&mut curwin_mut().w_cursor);
                    pos = findmatch(None, what);
                    if pos.is_null() {
                        curwin_mut().w_cursor = old_pos;
                        return false;
                    }
                    start_pos = *pos;
                    curwin_mut().w_cursor = *pos;
                    end_pos = findmatch(None, other);
                    if end_pos.is_null() {
                        curwin_mut().w_cursor = old_pos;
                        return false;
                    }
                    curwin_mut().w_cursor = *end_pos;
                } else {
                    break;
                }
            }
        }

        if g_st().visual_global.active {
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                inc(&mut curwin_mut().w_cursor);
            }
            if sol && gchar_cursor() != i32::from(NUL) {
                inc(&mut curwin_mut().w_cursor);
            }
            g_st().visual_global.start = start_pos;
            g_st().visual_global.mode = b'v' as i32;
            redraw_curbuf_later(UPD_INVERTED);
            showmode();
        } else {
            oap.start = start_pos;
            oap.motion_type = MT_CHAR_WISE;
            oap.inclusive = false;
            if sol {
                incl(&mut curwin_mut().w_cursor);
            } else if ltoreq(start_pos, curwin_mut().w_cursor) {
                oap.inclusive = true;
            } else {
                curwin_mut().w_cursor = start_pos;
            }
        }

        true
    }
}

/// Return true if the cursor is on a `<aaa>` tag (or `</aaa>` when `end_tag` is true) (`in_html_tag`).
fn in_html_tag(end_tag: bool) -> bool {
    unsafe {
        let line = get_cursor_line_ptr();
        let mut p = line.wrapping_add(curwin_mut().w_cursor.col as usize);
        let mut lc: i32 = i32::from(NUL);

        while p > line {
            if crate::memory::cstr_first_byte(p) == b'<' {
                break;
            }
            let off = utf_head_off(line, p.wrapping_sub(1)) + 1;
            p = p.wrapping_sub(off as usize);
            if crate::memory::cstr_first_byte(p) == b'>' {
                break;
            }
        }
        if crate::memory::cstr_first_byte(p) != b'<' {
            return false;
        }

        let mut pos = CursorPos {
            lnum: curwin_mut().w_cursor.lnum,
            col: p.offset_from(line) as Colnr,
            coladd: 0,
        };

        p = p.wrapping_add(utfc_ptr2len(p) as usize);
        if end_tag {
            return crate::memory::cstr_first_byte(p) == b'/';
        }

        if crate::memory::cstr_first_byte(p) == b'/' {
            return false;
        }

        loop {
            if inc(&mut pos) < 0 {
                return false;
            }
            let c = i32::from(*ml_get_pos(&pos) as u8);
            if c == b'>' as i32 {
                break;
            }
            lc = c;
        }
        lc != b'/' as i32
    }
}

/// Find tag block under the cursor, cursor at end (`current_tagblock`).
pub fn current_tagblock(oap: &mut Oparg, count_arg: i32, include: bool) -> bool {
    unsafe {
        let mut count = count_arg;
        let mut do_include = include;
        let save_p_ws = opt_vars().p_ws;
        let mut retval = false;
        let mut is_inclusive = true;

        opt_vars().p_ws = 0;

        let old_pos = curwin_mut().w_cursor;
        let mut old_end = curwin_mut().w_cursor;
        let mut old_start = old_end;
        if !g_st().visual_global.active || crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
        {
            decl(&mut old_end);
        }

        if !g_st().visual_global.active
            || equalpos(g_st().visual_global.start, curwin_mut().w_cursor)
        {
            setpcmark();

            while inindent(1) {
                if inc_cursor() != 0 {
                    break;
                }
            }

            if in_html_tag(false) {
                while crate::memory::cstr_first_byte(get_cursor_pos_ptr()) != b'>' {
                    if inc_cursor() < 0 {
                        break;
                    }
                }
            } else if in_html_tag(true) {
                while crate::memory::cstr_first_byte(get_cursor_pos_ptr()) != b'<' {
                    if dec_cursor() < 0 {
                        break;
                    }
                }
                dec_cursor();
                old_end = curwin_mut().w_cursor;
            }
        } else if lt(g_st().visual_global.start, curwin_mut().w_cursor) {
            old_start = g_st().visual_global.start;
            curwin_mut().w_cursor = g_st().visual_global.start;
        } else {
            old_end = g_st().visual_global.start;
        }

        'again: loop {
            for _ in 0..count {
                if do_searchpair(
                    c"<[^ \t>/!]\\+\\%(\\_s\\_[^>]\\{-}[^/]>\\|$\\|\\_s\\=>\\)",
                    c"",
                    c"</[^>]*>",
                    BACKWARD,
                    None,
                    0,
                    None,
                    0,
                    0,
                ) <= 0
                {
                    curwin_mut().w_cursor = old_pos;
                    opt_vars().p_ws = save_p_ws;
                    return retval;
                }
            }
            let mut start_pos = curwin_mut().w_cursor;

            inc_cursor();
            let p = get_cursor_pos_ptr();
            let mut cp = p;
            while !crate::memory::cstr_is_empty(cp)
                && crate::memory::cstr_first_byte(cp) != b'>'
                && !ascii_iswhite(i32::from(crate::memory::cstr_first_byte(cp)))
            {
                cp = cp.wrapping_add(utfc_ptr2len(cp) as usize);
            }
            let len = cp.offset_from(p) as usize;
            if len == 0 {
                curwin_mut().w_cursor = old_pos;
                opt_vars().p_ws = save_p_ws;
                return retval;
            }

            let tag_bytes = crate::memory::cstr_slice(p, len);
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
                c"",
                epat.as_ptr().cast::<c_char>(),
                FORWARD,
                None,
                0,
                None,
                0,
                0,
            );

            if r < 1 || lt(curwin_mut().w_cursor, old_end) {
                count = 1;
                curwin_mut().w_cursor = start_pos;
                continue 'again;
            }

            if do_include {
                while crate::memory::cstr_first_byte(get_cursor_pos_ptr()) != b'>' {
                    if inc_cursor() < 0 {
                        break;
                    }
                }
            } else {
                let c = get_cursor_pos_ptr();
                if crate::memory::cstr_first_byte(c) == b'<'
                    && !g_st().visual_global.active
                    && curwin_mut().w_cursor.col == 0
                {
                    is_inclusive = false;
                } else if crate::memory::cstr_first_byte(c) == b'<' {
                    dec_cursor();
                }
            }
            let end_pos = curwin_mut().w_cursor;

            if !do_include {
                let mut in_quotes = false;
                curwin_mut().w_cursor = start_pos;
                while inc_cursor() >= 0 {
                    let p_cur = get_cursor_pos_ptr();
                    if crate::memory::cstr_first_byte(p_cur) == b'>' && !in_quotes {
                        inc_cursor();
                        start_pos = curwin_mut().w_cursor;
                        break;
                    } else if crate::memory::cstr_first_byte(p_cur) == b'"'
                        || crate::memory::cstr_first_byte(p_cur) == b'\''
                    {
                        in_quotes = !in_quotes;
                    }
                }
                curwin_mut().w_cursor = end_pos;

                if g_st().visual_global.active
                    && equalpos(start_pos, old_start)
                    && equalpos(end_pos, old_end)
                {
                    do_include = true;
                    curwin_mut().w_cursor = old_start;
                    count = count_arg;
                    continue 'again;
                }
            }

            if g_st().visual_global.active {
                if lt(end_pos, start_pos) {
                    curwin_mut().w_cursor = start_pos;
                } else if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                    inc_cursor();
                }
                g_st().visual_global.start = start_pos;
                g_st().visual_global.mode = b'v' as i32;
                redraw_curbuf_later(UPD_INVERTED);
                showmode();
            } else {
                oap.start = start_pos;
                oap.motion_type = MT_CHAR_WISE;
                if lt(end_pos, start_pos) {
                    curwin_mut().w_cursor = start_pos;
                    oap.inclusive = false;
                } else {
                    oap.inclusive = is_inclusive;
                }
            }
            retval = true;
            break 'again;
        }

        opt_vars().p_ws = save_p_ws;
        retval
    }
}

/// Find paragraph under the cursor (`current_par`).
pub fn current_par(oap: &mut Oparg, count: i32, include: bool, type_: i32) -> bool {
    {
        let mut retval = true;
        let mut do_white = false;

        if type_ == b'S' as i32 {
            return false;
        }

        let mut start_lnum = curwin_mut().w_cursor.lnum;

        let do_extend =
            g_st().visual_global.active && start_lnum != g_st().visual_global.start.lnum;
        if !do_extend {
            let white_in_front = linewhite(start_lnum);
            while start_lnum > 1 {
                if white_in_front {
                    if !linewhite(start_lnum - 1) {
                        break;
                    }
                } else if linewhite(start_lnum - 1) || start_ps(start_lnum, 0, false) {
                    break;
                }
                start_lnum -= 1;
            }

            let mut end_lnum = start_lnum;
            while end_lnum <= curbuf_mut().b_ml.ml_line_count && linewhite(end_lnum) {
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
                if end_lnum == curbuf_mut().b_ml.ml_line_count {
                    return false;
                }

                if !include {
                    do_white = linewhite(end_lnum + 1);
                }

                if include || !do_white {
                    end_lnum += 1;
                    while end_lnum < curbuf_mut().b_ml.ml_line_count
                        && !linewhite(end_lnum + 1)
                        && !start_ps(end_lnum + 1, 0, false)
                    {
                        end_lnum += 1;
                    }
                }

                if i == 0 && white_in_front && include {
                    break;
                }

                if include || do_white {
                    while end_lnum < curbuf_mut().b_ml.ml_line_count && linewhite(end_lnum + 1) {
                        end_lnum += 1;
                    }
                }
            }

            if !white_in_front && !linewhite(end_lnum) && include {
                while start_lnum > 1 && linewhite(start_lnum - 1) {
                    start_lnum -= 1;
                }
            }

            if g_st().visual_global.active {
                if !(g_st().visual_global.mode == b'V' as i32
                    && start_lnum == curwin_mut().w_cursor.lnum)
                {
                    if g_st().visual_global.start.lnum != start_lnum {
                        g_st().visual_global.start.lnum = start_lnum;
                        g_st().visual_global.start.col = 0;
                    }
                    g_st().visual_global.mode = b'V' as i32;
                    redraw_curbuf_later(UPD_INVERTED);
                    showmode();
                    curwin_mut().w_cursor.lnum = end_lnum;
                    curwin_mut().w_cursor.col = 0;
                    return true;
                }
            } else {
                oap.start.lnum = start_lnum;
                oap.start.col = 0;
                oap.motion_type = MT_LINE_WISE;
                curwin_mut().w_cursor.lnum = end_lnum;
                curwin_mut().w_cursor.col = 0;
                return true;
            }
        }

        // extend:
        let dir = if start_lnum < g_st().visual_global.start.lnum {
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
                    curbuf_mut().b_ml.ml_line_count
                })
            {
                retval = false;
                break;
            }

            let mut prev_start_is_white: i32 = -1;
            for _ in 0..2 {
                start_lnum += dir;
                let start_is_white = linewhite(start_lnum);
                if prev_start_is_white == i32::from(start_is_white) {
                    start_lnum -= dir;
                    break;
                }
                loop {
                    if start_lnum
                        == (if dir == BACKWARD {
                            1
                        } else {
                            curbuf_mut().b_ml.ml_line_count
                        })
                    {
                        break;
                    }
                    if start_is_white != linewhite(start_lnum + dir)
                        || (!start_is_white
                            && start_ps(start_lnum + if dir > 0 { 1 } else { 0 }, 0, false))
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
                        curbuf_mut().b_ml.ml_line_count
                    })
                {
                    break;
                }
                prev_start_is_white = i32::from(start_is_white);
            }
        }
        curwin_mut().w_cursor.lnum = start_lnum;
        curwin_mut().w_cursor.col = 0;
        retval
    }
}

/// Search quote char from string `line[col]` (`find_next_quote`).
fn find_next_quote(
    line: impl crate::strings::IntoCCharPtr,
    mut col: i32,
    quotechar: i32,
    escape: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let line = line.into_c_char_ptr();
    let escape = escape.into_c_char_ptr();
    {
        loop {
            let c = i32::from(crate::memory::cstr_byte_at(line, col as usize));
            if c == i32::from(NUL) {
                return -1;
            } else if !escape.is_null() && crate::strings::vim_haschr(escape, c) {
                col += 1;
                if crate::memory::cstr_byte_at(line, col as usize) == 0 {
                    return -1;
                }
            } else if c == quotechar {
                break;
            }
            col += utfc_ptr2len(line.wrapping_add(col as usize));
        }
        col
    }
}

/// Search backwards in `line` from column `col_start` to find `quotechar` (`find_prev_quote`).
fn find_prev_quote(
    line: impl crate::strings::IntoCCharPtr,
    mut col_start: i32,
    quotechar: i32,
    escape: impl crate::strings::IntoCCharPtr,
) -> i32 {
    let line = line.into_c_char_ptr();
    let escape = escape.into_c_char_ptr();
    unsafe {
        while col_start > 0 {
            col_start -= 1;
            col_start -= utf_head_off(line, line.wrapping_add(col_start as usize));
            let mut n = 0;
            if !escape.is_null() {
                while col_start - n > 0
                    && crate::strings::vim_haschr(
                        escape,
                        i32::from(*line.wrapping_add((col_start - n - 1) as usize) as u8),
                    )
                {
                    n += 1;
                }
            }
            if (n & 1) != 0 {
                col_start -= n;
            } else if i32::from(crate::memory::cstr_byte_at(line, col_start as usize)) == quotechar
            {
                break;
            }
        }
        col_start
    }
}

/// Find quote under the cursor, cursor at end (`current_quote`).
pub fn current_quote(oap: &mut Oparg, count: i32, include: bool, quotechar: i32) -> bool {
    unsafe {
        let line = get_cursor_line_ptr();
        let mut col_end: i32;
        let mut col_start: i32 = curwin_mut().w_cursor.col;
        let mut inclusive = false;
        let mut vis_empty = true;
        let mut vis_bef_curs = false;
        let mut did_exclusive_adj = false;
        let mut inside_quotes = false;
        let mut selected_quote = false;
        let mut restore_vis_bef = false;

        let abort_search = |did_exclusive_adj: bool, restore_vis_bef: bool| -> bool {
            if g_st().visual_global.active
                && crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e'
            {
                if did_exclusive_adj {
                    inc_cursor();
                }
                if restore_vis_bef {
                    ptr::swap(
                        &raw mut curwin_mut().w_cursor,
                        &raw mut g_st().visual_global.start,
                    );
                }
            }
            false
        };

        if g_st().visual_global.active {
            if g_st().visual_global.start.lnum != curwin_mut().w_cursor.lnum {
                return false;
            }

            vis_bef_curs = lt(g_st().visual_global.start, curwin_mut().w_cursor);
            vis_empty = equalpos(g_st().visual_global.start, curwin_mut().w_cursor);
            if crate::memory::cstr_first_byte(opt_vars().p_sel) == b'e' {
                if vis_bef_curs {
                    dec_cursor();
                    did_exclusive_adj = true;
                } else if !vis_empty {
                    dec(&mut g_st().visual_global.start);
                    did_exclusive_adj = true;
                }
                vis_empty = equalpos(g_st().visual_global.start, curwin_mut().w_cursor);
                if !vis_bef_curs && !vis_empty {
                    ptr::swap(
                        &raw mut curwin_mut().w_cursor,
                        &raw mut g_st().visual_global.start,
                    );
                    vis_bef_curs = true;
                    restore_vis_bef = true;
                }
            }
        }

        if !vis_empty {
            let mut i: i32;
            if vis_bef_curs {
                inside_quotes = g_st().visual_global.start.col > 0
                    && i32::from(
                        *line.wrapping_add((g_st().visual_global.start.col - 1) as usize) as u8,
                    ) == quotechar
                    && *line.wrapping_add(curwin_mut().w_cursor.col as usize) != NUL
                    && i32::from(*line.wrapping_add((curwin_mut().w_cursor.col + 1) as usize) as u8)
                        == quotechar;
                i = g_st().visual_global.start.col;
                col_end = curwin_mut().w_cursor.col;
            } else {
                inside_quotes = curwin_mut().w_cursor.col > 0
                    && i32::from(*line.wrapping_add((curwin_mut().w_cursor.col - 1) as usize) as u8)
                        == quotechar
                    && *line.wrapping_add(g_st().visual_global.start.col as usize) != NUL
                    && i32::from(
                        *line.wrapping_add((g_st().visual_global.start.col + 1) as usize) as u8,
                    ) == quotechar;
                i = curwin_mut().w_cursor.col;
                col_end = g_st().visual_global.start.col;
            }

            while i <= col_end {
                if crate::memory::cstr_byte_at(line, i as usize) == 0 {
                    break;
                }
                let ch = i32::from(crate::memory::cstr_byte_at(line, i as usize));
                i += 1;
                if ch == quotechar {
                    selected_quote = true;
                    break;
                }
            }
        }

        if !vis_empty
            && i32::from(crate::memory::cstr_byte_at(line, col_start as usize)) == quotechar
        {
            if vis_bef_curs {
                col_start = find_next_quote(line, col_start + 1, quotechar, ptr::null_mut());
                if col_start < 0 {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                col_end = find_next_quote(line, col_start + 1, quotechar, curbuf_mut().b_p_qe);
                if col_end < 0 {
                    col_end = col_start;
                    col_start = curwin_mut().w_cursor.col;
                }
            } else {
                col_end = find_prev_quote(line, col_start, quotechar, ptr::null_mut());
                if i32::from(crate::memory::cstr_byte_at(line, col_end as usize)) != quotechar {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                col_start = find_prev_quote(line, col_end, quotechar, curbuf_mut().b_p_qe);
                if i32::from(crate::memory::cstr_byte_at(line, col_start as usize)) != quotechar {
                    col_start = col_end;
                    col_end = curwin_mut().w_cursor.col;
                }
            }
        } else if i32::from(crate::memory::cstr_byte_at(line, col_start as usize)) == quotechar
            || !vis_empty
        {
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
                col_end = find_next_quote(line, col_start + 1, quotechar, curbuf_mut().b_p_qe);
                if col_end < 0 {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
                if col_start <= first_col && first_col <= col_end {
                    break;
                }
                col_start = col_end + 1;
            }
        } else {
            col_start = find_prev_quote(line, col_start, quotechar, curbuf_mut().b_p_qe);
            if i32::from(crate::memory::cstr_byte_at(line, col_start as usize)) != quotechar {
                col_start = find_next_quote(line, col_start, quotechar, ptr::null_mut());
                if col_start < 0 {
                    return abort_search(did_exclusive_adj, restore_vis_bef);
                }
            }

            col_end = find_next_quote(line, col_start + 1, quotechar, curbuf_mut().b_p_qe);
            if col_end < 0 {
                return abort_search(did_exclusive_adj, restore_vis_bef);
            }
        }

        if include {
            if ascii_iswhite(i32::from(*line.wrapping_add((col_end + 1) as usize) as u8)) {
                while ascii_iswhite(i32::from(*line.wrapping_add((col_end + 1) as usize) as u8)) {
                    col_end += 1;
                }
            } else {
                while col_start > 0
                    && ascii_iswhite(i32::from(*line.wrapping_add((col_start - 1) as usize) as u8))
                {
                    col_start -= 1;
                }
            }
        }

        if !include && count < 2 && (vis_empty || !inside_quotes) {
            col_start += 1;
        }
        curwin_mut().w_cursor.col = col_start;
        if g_st().visual_global.active {
            if vis_empty
                || (vis_bef_curs
                    && !selected_quote
                    && (inside_quotes
                        || (i32::from(
                            *line.wrapping_add(g_st().visual_global.start.col as usize) as u8,
                        ) != quotechar
                            && (g_st().visual_global.start.col == 0
                                || i32::from(
                                    *line
                                        .wrapping_add((g_st().visual_global.start.col - 1) as usize)
                                        as u8,
                                ) != quotechar))))
            {
                g_st().visual_global.start = curwin_mut().w_cursor;
                redraw_curbuf_later(UPD_INVERTED);
            }
        } else {
            oap.start = curwin_mut().w_cursor;
            oap.motion_type = MT_CHAR_WISE;
        }

        curwin_mut().w_cursor.col = col_end;
        if (include || count > 1 || (!vis_empty && inside_quotes)) && inc_cursor() == 2 {
            inclusive = true;
        }
        if g_st().visual_global.active {
            if vis_empty || vis_bef_curs {
                if crate::memory::cstr_first_byte(opt_vars().p_sel) != b'e' {
                    dec_cursor();
                }
            } else {
                if inside_quotes
                    || (!selected_quote
                        && i32::from(
                            *line.wrapping_add(g_st().visual_global.start.col as usize) as u8
                        ) != quotechar
                        && (*line.wrapping_add(g_st().visual_global.start.col as usize) == NUL
                            || i32::from(
                                *line.wrapping_add((g_st().visual_global.start.col + 1) as usize)
                                    as u8,
                            ) != quotechar))
                {
                    dec_cursor();
                    g_st().visual_global.start = curwin_mut().w_cursor;
                }
                curwin_mut().w_cursor.col = col_start;
            }
            if g_st().visual_global.mode == b'V' as i32 {
                g_st().visual_global.mode = b'v' as i32;
                g_st().redraw_cmdline = true;
            }
        } else {
            oap.inclusive = inclusive;
        }

        true
    }
}
