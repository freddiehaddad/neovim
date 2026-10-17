#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "nvim/buffer_defs.h"  // buf_T
#include "nvim/eval/typval_defs.h"  // varnumber_T
#include "nvim/input_cmdatom_defs.h"  // IWYU pragma: export
#include "nvim/normal_defs.h"  // VisualState, cmdarg_T
#include "nvim/pos_defs.h"
#include "nvim/register_defs.h"  // Timestamp

/// Multicursor: pending atoms; they cascade as a batch (mc_clock_edge).
extern CmdAtomVec g_atoms;

/// One `normal_execute()`: the scope of a MODE_NORMAL command (Normal/Visual/Select/Op-pending, see
/// get_real_state()). Same lifetime as `cmdarg_T`. A `composite` spans successive toplevel frames.
///
/// Entry state (`origin`, …) is diffed at `atom_cmd_end()` into the command's CmdAtom.
///
/// Insert/Cmdline modes (non-MODE_NORMAL) are "sessions", not "frames" (atom_ins_start(),
/// atom_payload_start()). Mappings initiating from non-MODE_NORMAL defer their `composite` to the
/// first frame.
typedef struct CmdFrame CmdFrame;
struct CmdFrame {
  CmdOrigin origin;     ///< State at entry.
  CmdAtom staged;       ///< Atom staged in this frame (`keys=NULL`: none).
  CmdFrame *parent;     ///< Enclosing frame (nested normal_execute()); NULL at toplevel.
  VisualState visual;   ///< Visual-mode state (active/start/mode are diffed).

  bool keytyped;        ///< KeyTyped
  unsigned keyclass;    ///< atom_key_class() of the cmd char, before its `nchar`.
  int ex_normal;        ///< If higher than `root_frame().ex_normal`, a cmd fed this (:norm, feed).
  uint64_t captures;    ///< `atom_captures` at entry.
  uint64_t global_ops;  ///< `global_ops` at entry.
  uint64_t beeps;       ///< `did_beep` at entry.
  uint64_t id;          ///< Identifies this frame (see `composite.frame`).
  bool follow;          ///< Follow-mode: updated until cursor-move happens (sticky) in this frame.
  bool consumers;       ///< Capture is skipped if there are no consumers (for performance).
  Timestamp reg_ts;     ///< Max register timestamp (to detect a per-cursor register write).
  size_t payload_start;  ///< Payload slice (SIZE_MAX: none): typed.keys[payload_start..payload_end)
  size_t payload_end;

  //
  // Toplevel command state: root_frame() only.
  //
  uint64_t redo_frame;  ///< Frame whose redobuf (potentially) defines the atom. 0: none.
  char *cmdline;        ///< The ":" payload captured at cmdline accept. NULL: none.
                        ///< Note: search payloads ("/pat<CR>") travel on `cmdarg.searchbuf`.
};

#ifndef DLLEXPORT
# ifdef MSWIN
#  define DLLEXPORT __declspec(dllexport)
# else
#  define DLLEXPORT
# endif
#endif

DLLEXPORT void atom_free(CmdAtom *atom);
DLLEXPORT void atoms_free(CmdAtomVec *v);
DLLEXPORT void atom_free_all(void);
DLLEXPORT CmdSpec atom_cmd_spec(const cmdarg_T *cap);
DLLEXPORT pos_T atom_origin_pos(buf_T *buf);
DLLEXPORT void atom_push_raw(bool cascade, CmdAtom *atom);
DLLEXPORT void atom_lhs_replay_queue(void);
DLLEXPORT bool atom_composite_queued(void);
DLLEXPORT bool atom_composite_active(void);
DLLEXPORT void atom_composite_abort(void);
DLLEXPORT void atom_term_enter(void);
DLLEXPORT bool atom_is_user_cmd(void);
DLLEXPORT void atom_suppress(bool suppress);
DLLEXPORT unsigned atom_key_class(int cmd, int arg, int arg2);
DLLEXPORT void atom_cmdline_set(int firstc, const char *line, size_t len);
DLLEXPORT void atom_payload_start(void);
DLLEXPORT void atom_payload_end(void);
DLLEXPORT void atom_typed_add(const uint8_t *chars, size_t len);
DLLEXPORT void atom_typed_del(size_t len);
DLLEXPORT void atom_did_global_op(void);
DLLEXPORT void atom_follow_changed(void);
DLLEXPORT void atom_redo_prepped(void);
DLLEXPORT void atom_redo_cancel(void);
DLLEXPORT void atom_macro_start(int regname);
DLLEXPORT void atom_stuff_start(const cmdarg_T *cap);
DLLEXPORT void atom_map_start(const char *lhs, size_t len, bool peeked);
DLLEXPORT void atom_visual_reset(void);
DLLEXPORT bool atom_visual_replayable(void);
DLLEXPORT bool atom_visual_redoable(void);
DLLEXPORT String atom_visual_span(void);
DLLEXPORT bool atom_visual_end(CmdSpec spec, bool redoable);
DLLEXPORT void atom_capture_op(oparg_T *oap, cmdarg_T *cap, bool redo_yank);
DLLEXPORT InsSession atom_ins_start(int cmd, long count, VisualIns vis, bool vblock);
DLLEXPORT void atom_ins_end(InsSession *session, bool busy);
DLLEXPORT void atom_cmd_start(CmdFrame *old, int cmdchar);
DLLEXPORT void atom_cmd_end(cmdarg_T *ca, CmdFrame *old);
