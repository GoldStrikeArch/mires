//! `js!{}`: lowering a block of JavaScript written in Rust source.
//!
//! A program writes one with the `js-macro` function macro, which lexes the block, numbers its free
//! identifiers and writes a marker function carrying the result in its `link_section`. The call is
//! intercepted in [`crate::abi`]'s `codegen_call` for the same reason a `#[js_extern]` declaration
//! is: the marker is an ordinary Rust function with a body, so without the interception the call
//! would resolve and emit a jump to an `unreachable!`.
//!
//! # What is emitted
//!
//! The block's text is [`Expr::Raw`], because there is no JavaScript parser in this backend and the
//! text is not this backend's to understand. The captures are bound by an arrow that is called
//! immediately:
//!
//! ```text
//! ((_$js0, _$js1) => globalThis.fetch(_$js0, { body: _$js1 }))(url, body)
//! ```
//!
//! An arrow rather than a textual splice of the argument expressions, and the reason is that an
//! argument is an arbitrary expression: splicing it would evaluate it once per occurrence, in
//! whatever order the block happens to name its captures, and would need this backend to print an
//! expression to text and re-parenthesize it. Binding is the operation the block wants, and
//! JavaScript already has it. A block with no captures needs no arrow and gets none.
//!
//! # What it costs
//!
//! `crate::minify`'s local pass refuses to rename anything in a function holding verbatim
//! JavaScript, because it cannot see which names the text mentions. A function containing a `js!{}`
//! block therefore keeps its long local names. That is a real cost and the alternative is a
//! JavaScript parser; hoisting a block into an item of its own would confine the cost to that item,
//! and is the obvious next step if it starts to matter.

use rustc_middle::mir;
use rustc_middle::ty::TyCtxt;
use rustc_span::Spanned;

use crate::base::FnCx;
use crate::block::{self, Block, PREFIX};
use crate::jsast::{self, Expr, Stmt};

/// The block `def_id` carries, or `None` for anything that is not a `js!{}` marker.
pub(crate) fn block_section(tcx: TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section.as_str().strip_prefix(PREFIX).map(str::to_owned)
}

impl<'tcx> FnCx<'_, 'tcx> {
    /// Lowers a call to a `js!{}` marker.
    pub(crate) fn codegen_js_block(
        &self,
        body: &str,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let block = match Block::decode(body) {
            Ok(block) => block,
            Err(why) => return vec![jsast::expr_stmt(self.zombie(why))],
        };

        // The macro writes one argument per slot, so a mismatch means the two halves disagree about
        // the format. Named rather than emitted, exactly as a malformed section is.
        if args.len() != block.slots {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "`{PREFIX}{body}` takes {} capture(s) and was called with {}",
                block.slots,
                args.len()
            )))];
        }

        let value = match block.slots {
            0 => jsast::raw_expr(block.text),
            slots => {
                let captures: Vec<Expr> = args
                    .iter()
                    .map(|arg| self.at(arg.span, || self.codegen_operand(&arg.node)))
                    .collect();
                let params = (0..slots).map(block::slot_name).collect();
                jsast::call(jsast::arrow(params, jsast::raw_expr(block.text)), captures)
            }
        };

        self.write_place(destination, value)
    }
}
