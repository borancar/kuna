//! (kuna `impliedtest`) Remove a compare-cascade test the next test makes
//! redundant.
//!
//! MSVC lowers a switch on `v` with one case `k` and the other values in
//! `default` (`case 0:` included) to `sub eax,0; je D; sub eax,k; jne D`. Both
//! tests survive to structuring and print as `if ((v) && (v == k))`, though
//! `v == k` with `k != 0` already rules out `v == 0`.
//!
//! The pattern: block A goes to D when `v == a` and otherwise to B; B has A as
//! its only predecessor, holds nothing but the arithmetic of its own test, and
//! goes to D when `v != b`, for a constant `b != a`. Every value A sends to D
//! reaches D through B as well, so A's edge to D is removed, provided each phi
//! in D carries the same value on both edges. The rewrite drops a compare the
//! binary performs, so it is an option.

use kuna_num::opcodes::OpCode::{self, *};

use crate::action::{Action, ActionBase, ActionContext, ActionGroupList, ApplyResult};
use crate::context::{BlockId, VarnodeId};
use crate::funcdata::Funcdata;
use crate::kuna_devectorize::{basic, constant, def, op, strip, taken};
use kuna_base::types::int4;

/// Ops a test block may hold besides its branch: the arithmetic of the test.
const TEST_OPS: &[OpCode] = &[
    CPUI_COPY,
    CPUI_INT_ADD,
    CPUI_INT_SUB,
    CPUI_INT_ZEXT,
    CPUI_INT_SEXT,
    CPUI_SUBPIECE,
    CPUI_INT_EQUAL,
    CPUI_INT_NOTEQUAL,
    CPUI_BOOL_NEGATE,
];

pub struct ActionImpliedTest {
    base: ActionBase,
    /// Unit-test override of the `kuna_impliedtest` gate; the scheduled
    /// instance passes `false` and reads the live flag.
    enabled: bool,
}

impl ActionImpliedTest {
    pub fn new(enabled: bool, g: impl Into<String>) -> Self {
        Self { base: ActionBase::new(0, "impliedtest", g), enabled }
    }
}

impl Action for ActionImpliedTest {
    fn base(&self) -> &ActionBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ActionBase {
        &mut self.base
    }

    fn clone_filtered(&self, grouplist: &ActionGroupList) -> Option<Box<dyn Action>> {
        grouplist
            .contains(self.get_group())
            .then(|| Box::new(Self { base: self.base.clone(), enabled: self.enabled }) as Box<dyn Action>)
    }

    fn apply(&mut self, data: &mut Funcdata, _ctx: &mut ActionContext) -> ApplyResult {
        if !self.enabled && !data.get_arch().kuna_impliedtest {
            return 0;
        }
        for i in 0..data.bblocks_get_size() {
            let a = data.bblocks_get_block(i);
            let Some(edge) = redundant_edge(data, a) else { continue };
            data.remove_branch(a, edge).expect("impliedtest: remove_branch");
            data.remove_unreachable_blocks(false, true).expect("impliedtest: remove_unreachable_blocks");
            self.base.count += 1;
            return 1;
        }
        0
    }
}

/// For a block ending in `v == k` or `v != k` with `k` constant, the tested
/// value, the constant, and the successors taken when they agree and differ.
fn equality_test(data: &Funcdata, b: BlockId) -> Option<(VarnodeId, u64, BlockId, BlockId)> {
    let (t, f) = taken(data, b)?;
    let (_, cond) = def(data, op(data, data.bb_op_tail(b)?).get_in(1)?)?;
    let equal = match cond.code() {
        CPUI_INT_EQUAL => true,
        CPUI_INT_NOTEQUAL => false,
        _ => return None,
    };
    let (x, y) = (cond.get_in(0)?, cond.get_in(1)?);
    let (v, k) = match (constant(data, x), constant(data, y)) {
        (None, Some(k)) => (strip(data, x, false), k),
        (Some(k), None) => (strip(data, y, false), k),
        _ => return None,
    };
    Some(if equal { (v, k, t, f) } else { (v, k, f, t) })
}

/// Whether every op of `b` but its branch is test arithmetic read only in `b`.
fn test_only(data: &Funcdata, b: BlockId) -> bool {
    let tail = data.bb_op_tail(b);
    data.bb_ops(b).into_iter().all(|o| {
        if Some(o) == tail {
            return true;
        }
        let code = op(data, o);
        TEST_OPS.contains(&code.code())
            && code.get_out().and_then(|v| data.vbank().get(v)).is_some_and(|v| {
                !v.is_persist()
                    && !v.is_addr_tied()
                    && v.descend_iter().all(|d| data.obank().get(d).and_then(|d| d.get_parent()) == Some(b))
            })
    })
}

/// The out-edge index of A's test that the next test makes redundant.
fn redundant_edge(data: &Funcdata, a: BlockId) -> Option<int4> {
    if !basic(data, a) {
        return None;
    }
    let (v, ka, d, b) = equality_test(data, a)?;
    if b == d || b == a || !basic(data, b) || data.bblocks_ref().block(b).size_in() != 1 || !test_only(data, b) {
        return None;
    }
    let (vb, kb, b_equal, b_differ) = equality_test(data, b)?;
    if vb != v || kb == ka || b_differ != d || b_equal == d {
        return None;
    }
    let dblock = data.bblocks_ref().block(d);
    let (from_a, from_b) = (dblock.get_in_index(a), dblock.get_in_index(b));
    if from_a < 0 || from_b < 0 {
        return None;
    }
    let same_values = data.bb_ops(d).into_iter().all(|o| {
        let phi = op(data, o);
        phi.code() != CPUI_MULTIEQUAL || phi.get_in(from_a) == phi.get_in(from_b)
    });
    if !same_values {
        return None;
    }
    let ablock = data.bblocks_ref().block(a);
    (0..ablock.size_out()).find(|&j| ablock.get_out(j) == d)
}
