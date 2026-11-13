//! (kuna) Pin the stack after a call of unknown pop from the `ret` it reaches.
//!
//! `StackSolver` takes a call whose extrapop the model leaves unknown as a
//! *guess*, and for an indirect callee the guess is that it pops nothing.
//! Every MSVC virtual call is `__thiscall` and pops its arguments, so after
//! `push edi; push ebx; push eax; push -1; push -1; call edx` the solved stack
//! pointer is 0x14 too low for the rest of the function: a later `lea
//! ecx,[esp+0x10]` names a slot below the frame object it points at, the alias
//! test then refuses the call's own last argument, and the `/GS` cookie pushed
//! above the arguments can take its place.
//!
//! The caller's own return settles it. An x86 `ret` reads its return address
//! at the stack pointer, so at that instruction the stack pointer is the one
//! the function was entered with. Walking back from the `ret` over the raw
//! p-code, into every predecessor of a block (the depth is the same on each
//! edge into it), every write of the stack pointer by a constant and every
//! call whose pop is known moves the depth by a known amount (a call to an
//! address inside the function's own extent is a jump in disguise, and ends
//! the path), until the first
//! call whose pop is unknown: when that call goes through a target computed
//! from registers (a virtual call, a function pointer held in a register or a
//! structure), the depth right after it is recorded on its call spec. A call
//! through a constant slot (an import, a global function pointer) has a guess
//! of its own and ends the path unrecorded. The pinned depth makes the solved
//! pop absorb every error of the guesses before the call, so a function records
//! nothing unless every call whose pop rests on the bare "pops nothing" guess
//! (a direct call with no purge read, an indirect call through a computed
//! target) is pinned this way; an import's push run is a better reading than
//! the pin. A write of the stack pointer by anything else ends a path,
//! and a call two paths reach at different depths records nothing. [`crate::coreaction_stackptr`] turns the record
//! into the first guess for that call, so it applies only where the exact
//! equations leave the stack open, and the call's pop is then solved instead
//! of guessed, and [`crate::funcdata_callsite::check_input_trial_use`] reads
//! the solved pop where upstream reads only the prototype's: the call's stack
//! trials wait one pass for it, then the ones it pops are its arguments and
//! the slots above them, the `/GS` cookie among them, are not.

use std::collections::{HashMap, HashSet};

use kuna_base::types::{int4, uintb};
use kuna_num::opcodes::OpCode;

use crate::context::{BlockId, OpId};
use crate::fspec::EXTRAPOP_UNKNOWN;
use crate::funcdata::Funcdata;

/// Ops walked back from one return before the walk gives up.
const BUDGET: usize = 512;

/// Record, on each call of unknown pop that a return is reached from by a
/// known stack walk, the entry-relative stack depth right after the call.
pub fn record(data: &mut Funcdata) {
    let Some(sp) = data
        .get_arch()
        .manage()
        .get_stack_space()
        .and_then(|s| s.get_spacebase(0).ok())
    else {
        return;
    };
    let Some(sp_space) = sp.space.clone() else { return };
    let is_sp = |data: &Funcdata, v: Option<crate::context::VarnodeId>| {
        v.and_then(|v| data.vbank().get(v)).is_some_and(|v| {
            v.get_size() as uintb == sp.size as uintb
                && v.get_addr().get_offset() == sp.offset
                && v.get_addr().get_space().is_some_and(|s| s.get_index() == sp_space.get_index())
        })
    };
    let extent = data
        .obank()
        .iter_all()
        .map(|(seq, _)| seq.get_addr().get_offset())
        .fold(None, |acc: Option<(uintb, uintb)>, at| Some(acc.map_or((at, at), |(lo, hi)| (lo.min(at), hi.max(at)))));
    let mut depths: HashMap<int4, Option<int4>> = HashMap::new();
    let returns: Vec<OpId> = data.obank().iter_code(OpCode::CPUI_RETURN).collect();
    for ret in returns {
        let Some(at) = data.obank().get(ret).map(|o| o.get_addr().clone()) else { continue };
        let mut first = ret;
        while let Some(prev) = data.op_previous_op(first).filter(|&p| data.obank().get(p).is_some_and(|o| o.get_addr() == &at)) {
            first = prev;
        }
        let mut work: Vec<(OpId, int4)> = Vec::new();
        let mut seen: HashSet<BlockId> = HashSet::new();
        enter_predecessors(data, first, 0, &mut work, &mut seen);
        let mut steps = 0;
        'path: while let Some((start, mut depth)) = work.pop() {
            let mut cur = Some(start);
            while let Some(op) = cur {
                steps += 1;
                if steps > BUDGET {
                    break 'path;
                }
                let o = data.obank().get(op).expect("returndepth: op");
                match o.code() {
                    OpCode::CPUI_CALL | OpCode::CPUI_CALLIND => {
                        let Some(idx) = data.get_call_specs_index(op) else { continue 'path };
                        let fc = data.get_call_specs(idx);
                        if extent.as_ref().is_some_and(|(lo, hi)| {
                            let at = fc.get_entry_address().get_offset();
                            *lo <= at && at <= *hi
                        }) {
                            continue 'path;
                        }
                        let pop = fc.callee_purge().or_else(|| {
                            fc.proto()
                                .has_model()
                                .then(|| fc.get_model_extra_pop())
                                .filter(|&model| model != EXTRAPOP_UNKNOWN)
                        });
                        match pop {
                            Some(p) => depth -= p,
                            None if !computed_target(data, op) => continue 'path,
                            None => {
                                let entry = depths.entry(idx).or_insert(Some(depth));
                                if *entry != Some(depth) {
                                    *entry = None;
                                }
                                continue 'path;
                            }
                        }
                    }
                    _ if is_sp(data, o.get_out()) => {
                        let c = o
                            .get_in(1)
                            .and_then(|v| data.vbank().get(v))
                            .filter(|v| v.is_constant())
                            .map(|v| v.get_offset() as u32 as i32);
                        match (o.code(), is_sp(data, o.get_in(0)), c) {
                            (OpCode::CPUI_INT_ADD, true, Some(c)) => depth -= c,
                            (OpCode::CPUI_INT_SUB, true, Some(c)) => depth += c,
                            _ => continue 'path,
                        }
                    }
                    _ => {}
                }
                cur = data.op_previous_op(op);
                if cur.is_none() {
                    enter_predecessors(data, op, depth, &mut work, &mut seen);
                }
            }
        }
    }
    let guessed = (0..data.num_calls()).filter(|&idx| {
        let fc = data.get_call_specs(idx);
        let known = fc.callee_purge().is_some()
            || (fc.proto().has_model() && fc.get_model_extra_pop() != EXTRAPOP_UNKNOWN);
        !known && (computed_target(data, fc.get_op()) || data.obank().get(fc.get_op()).is_some_and(|o| o.code() == OpCode::CPUI_CALL))
    });
    let all_pinned = guessed.into_iter().all(|idx| depths.get(&idx).is_some_and(|d| d.is_some()));
    if !all_pinned {
        return;
    }
    for (idx, depth) in depths {
        if let Some(d) = depth {
            data.get_call_specs_mut(idx).set_return_depth(Some(d));
        }
    }
}

/// Queue the op before `op`, or, when `op` starts its block, the last op of
/// each predecessor not yet entered: the stack depth is the same on every edge
/// into a block.
fn enter_predecessors(data: &Funcdata, op: OpId, depth: int4, work: &mut Vec<(OpId, int4)>, seen: &mut HashSet<BlockId>) {
    if let Some(prev) = data.op_previous_op(op) {
        work.push((prev, depth));
        return;
    }
    let Some(block) = data.obank().get(op).and_then(|o| o.get_parent()) else { return };
    let fb = data.bblocks_ref().block(block);
    for k in 0..fb.size_in() {
        let pred = fb.get_in(k);
        if seen.insert(pred) {
            if let Some(tail) = data.bb_op_tail(pred) {
                work.push((tail, depth));
            }
        }
    }
}

/// Whether `op` is an indirect call through a target computed from registers
/// (a virtual call, a function pointer in a register or a structure), rather
/// than a direct call or a call through a constant slot (an import, a global
/// function pointer), whose pop has a guess of its own.
fn computed_target(data: &Funcdata, op: OpId) -> bool {
    let Some(o) = data.obank().get(op).filter(|o| o.code() == OpCode::CPUI_CALLIND) else { return false };
    let Some(target) = o.get_in(0).and_then(|v| data.vbank().get(v)) else { return false };
    let space = |v: &crate::varnode::Varnode| v.get_addr().get_space().map(|s| (s.get_type(), s.get_name() == "register"));
    let computed = |v: &crate::varnode::Varnode| match space(v) {
        Some((_, true)) => true,
        Some((kuna_base::space::spacetype::IPTR_INTERNAL, _)) => true,
        _ => false,
    };
    if space(target) != Some((kuna_base::space::spacetype::IPTR_INTERNAL, false)) {
        return computed(target);
    }
    let mut prev = data.op_previous_op(op);
    while let Some(p) = prev.and_then(|p| data.obank().get(p)).filter(|p| p.get_addr() == o.get_addr()) {
        let writes_target = p
            .get_out()
            .and_then(|v| data.vbank().get(v))
            .is_some_and(|v| v.get_addr() == target.get_addr() && v.get_size() == target.get_size());
        if writes_target {
            let input = |k| p.get_in(k).and_then(|v| data.vbank().get(v));
            return match p.code() {
                OpCode::CPUI_LOAD => input(1).is_some_and(|v| !v.is_constant()),
                OpCode::CPUI_COPY => input(0).is_some_and(computed),
                _ => true,
            };
        }
        prev = prev.and_then(|q| data.op_previous_op(q));
    }
    true
}
