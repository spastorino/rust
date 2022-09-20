use std::cell::Cell;
use std::fmt;
use std::mem;

use rustc_hir::{self as hir, def_id::DefId, definitions::DefPathData};
use rustc_index::vec::IndexVec;
use rustc_middle::mir;
use rustc_middle::mir::interpret::{InterpError, InvalidProgramInfo};
use rustc_middle::ty::layout::{
    self, FnAbiError, FnAbiOfHelpers, FnAbiRequest, LayoutError, LayoutOf, LayoutOfHelpers,
    TyAndLayout,
};
use rustc_middle::ty::{
    self, query::TyCtxtAt, subst::SubstsRef, ParamEnv, Ty, TyCtxt, TypeFoldable,
};
use rustc_mir_dataflow::storage::always_storage_live_locals;
use rustc_session::Limit;
use rustc_span::{Pos, Span};
use rustc_target::abi::{call::FnAbi, Align, HasDataLayout, Size, TargetDataLayout};

use super::{
    AllocId, GlobalId, Immediate, InterpErrorInfo, InterpResult, MPlaceTy, Machine, MemPlace,
    MemPlaceMeta, Memory, MemoryKind, Operand, Place, PlaceTy, PointerArithmetic, Provenance,
    Scalar, StackPopJump,
};
use crate::transform::validate::equal_up_to_regions;

pub struct InterpCx<'mir, 'tcx, M: Machine<'mir, 'tcx>> {
    /// Stores the `Machine` instance.
    ///
    /// Note: the stack is provided by the machine.
    pub machine: M,

    /// The results of the type checker, from rustc.
    /// The span in this is the "root" of the evaluation, i.e., the const
    /// we are evaluating (if this is CTFE).
    pub tcx: TyCtxtAt<'tcx>,

    /// Bounds in scope for polymorphic evaluations.
    pub(crate) param_env: ty::ParamEnv<'tcx>,

    /// The virtual memory system.
    pub memory: Memory<'mir, 'tcx, M>,

    /// The recursion limit (cached from `tcx.recursion_limit(())`)
    pub recursion_limit: Limit,
}

// The Phantomdata exists to prevent this type from being `Send`. If it were sent across a thread
// boundary and dropped in the other thread, it would exit the span in the other thread.
struct SpanGuard(tracing::Span, std::marker::PhantomData<*const u8>);

impl SpanGuard {
    /// By default a `SpanGuard` does nothing.
    fn new() -> Self {
        Self(tracing::Span::none(), std::marker::PhantomData)
    }

    /// If a span is entered, we exit the previous span (if any, normally none) and enter the
    /// new span. This is mainly so we don't have to use `Option` for the `tracing_span` field of
    /// `Frame` by creating a dummy span to being with and then entering it once the frame has
    /// been pushed.
    fn enter(&mut self, span: tracing::Span) {
        // This executes the destructor on the previous instance of `SpanGuard`, ensuring that
        // we never enter or exit more spans than vice versa. Unless you `mem::leak`, then we
        // can't protect the tracing stack, but that'll just lead to weird logging, no actual
        // problems.
        *self = Self(span, std::marker::PhantomData);
        self.0.with_subscriber(|(id, dispatch)| {
            dispatch.enter(id);
        });
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        self.0.with_subscriber(|(id, dispatch)| {
            dispatch.exit(id);
        });
    }
}

/// A stack frame.
pub struct Frame<'mir, 'tcx, Prov: Provenance = AllocId, Extra = ()> {
    ////////////////////////////////////////////////////////////////////////////////
    // Function and callsite information
    ////////////////////////////////////////////////////////////////////////////////
    /// The MIR for the function called on this frame.
    pub body: &'mir mir::Body<'tcx>,

    /// The def_id and substs of the current function.
    pub instance: ty::Instance<'tcx>,

    /// Extra data for the machine.
    pub extra: Extra,

    ////////////////////////////////////////////////////////////////////////////////
    // Return place and locals
    ////////////////////////////////////////////////////////////////////////////////
    /// Work to perform when returning from this function.
    pub return_to_block: StackPopCleanup,

    /// The location where the result of the current stack frame should be written to,
    /// and its layout in the caller.
    pub return_place: PlaceTy<'tcx, Prov>,

    /// The list of locals for this stack frame, stored in order as
    /// `[return_ptr, arguments..., variables..., temporaries...]`.
    /// The locals are stored as `Option<Value>`s.
    /// `None` represents a local that is currently dead, while a live local
    /// can either directly contain `Scalar` or refer to some part of an `Allocation`.
    ///
    /// Do *not* access this directly; always go through the machine hook!
    pub locals: IndexVec<mir::Local, LocalState<'tcx, Prov>>,

    /// The span of the `tracing` crate is stored here.
    /// When the guard is dropped, the span is exited. This gives us
    /// a full stack trace on all tracing statements.
    tracing_span: SpanGuard,

    ////////////////////////////////////////////////////////////////////////////////
    // Current position within the function
    ////////////////////////////////////////////////////////////////////////////////
    /// If this is `Err`, we are not currently executing any particular statement in
    /// this frame (can happen e.g. during frame initialization, and during unwinding on
    /// frames without cleanup code).
    /// We basically abuse `Result` as `Either`.
    ///
    /// Needs to be public because ConstProp does unspeakable things to it.
    pub loc: Result<mir::Location, Span>,
}

/// What we store about a frame in an interpreter backtrace.
#[derive(Debug)]
pub struct FrameInfo<'tcx> {
    pub instance: ty::Instance<'tcx>,
    pub span: Span,
    pub lint_root: Option<hir::HirId>,
}

/// Unwind information.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum StackPopUnwind {
    /// The cleanup block.
    Cleanup(mir::BasicBlock),
    /// No cleanup needs to be done.
    Skip,
    /// Unwinding is not allowed (UB).
    NotAllowed,
}

#[derive(Clone, Copy, Eq, PartialEq, Debug)] // Miri debug-prints these
pub enum StackPopCleanup {
    /// Jump to the next block in the caller, or cause UB if None (that's a function
    /// that may never return). Also store layout of return place so
    /// we can validate it at that layout.
    /// `ret` stores the block we jump to on a normal return, while `unwind`
    /// stores the block used for cleanup during unwinding.
    Goto { ret: Option<mir::BasicBlock>, unwind: StackPopUnwind },
    /// The root frame of the stack: nowhere else to jump to.
    /// `cleanup` says whether locals are deallocated. Static computation
    /// wants them leaked to intern what they need (and just throw away
    /// the entire `ecx` when it is done).
    Root { cleanup: bool },
}

/// State of a local variable including a memoized layout
#[derive(Clone, Debug)]
pub struct LocalState<'tcx, Prov: Provenance = AllocId> {
    pub value: LocalValue<Prov>,
    /// Don't modify if `Some`, this is only used to prevent computing the layout twice
    pub layout: Cell<Option<TyAndLayout<'tcx>>>,
}

/// Current value of a local variable
#[derive(Copy, Clone, Debug)] // Miri debug-prints these
pub enum LocalValue<Prov: Provenance = AllocId> {
    /// This local is not currently alive, and cannot be used at all.
    Dead,
    /// A normal, live local.
    /// Mostly for convenience, we re-use the `Operand` type here.
    /// This is an optimization over just always having a pointer here;
    /// we can thus avoid doing an allocation when the local just stores
    /// immediate values *and* never has its address taken.
    Live(Operand<Prov>),
}

impl<'tcx, Prov: Provenance + 'static> LocalState<'tcx, Prov> {
    /// Read the local's value or error if the local is not yet live or not live anymore.
    #[inline]
    pub fn access(&self) -> InterpResult<'tcx, &Operand<Prov>> {
        match &self.value {
            LocalValue::Dead => throw_ub!(DeadLocal), // could even be "invalid program"?
            LocalValue::Live(val) => Ok(val),
        }
    }

    /// Overwrite the local.  If the local can be overwritten in place, return a reference
    /// to do so; otherwise return the `MemPlace` to consult instead.
    ///
    /// Note: This may only be invoked from the `Machine::access_local_mut` hook and not from
    /// anywhere else. You may be invalidating machine invariants if you do!
    #[inline]
    pub fn access_mut(&mut self) -> InterpResult<'tcx, &mut Operand<Prov>> {
        match &mut self.value {
            LocalValue::Dead => throw_ub!(DeadLocal), // could even be "invalid program"?
            LocalValue::Live(val) => Ok(val),
        }
    }
}

impl<'mir, 'tcx, Prov: Provenance> Frame<'mir, 'tcx, Prov> {
    pub fn with_extra<Extra>(self, extra: Extra) -> Frame<'mir, 'tcx, Prov, Extra> {
        Frame {
            body: self.body,
            instance: self.instance,
            return_to_block: self.return_to_block,
            return_place: self.return_place,
            locals: self.locals,
            loc: self.loc,
            extra,
            tracing_span: self.tracing_span,
        }
    }
}

impl<'mir, 'tcx, Prov: Provenance, Extra> Frame<'mir, 'tcx, Prov, Extra> {
    /// Get the current location within the Frame.
    ///
    /// If this is `Err`, we are not currently executing any particular statement in
    /// this frame (can happen e.g. during frame initialization, and during unwinding on
    /// frames without cleanup code).
    /// We basically abuse `Result` as `Either`.
    ///
    /// Used by priroda.
    pub fn current_loc(&self) -> Result<mir::Location, Span> {
        self.loc
    }

    /// Return the `SourceInfo` of the current instruction.
    pub fn current_source_info(&self) -> Option<&mir::SourceInfo> {
        self.loc.ok().map(|loc| self.body.source_info(loc))
    }

    pub fn current_span(&self) -> Span {
        match self.loc {
            Ok(loc) => self.body.source_info(loc).span,
            Err(span) => span,
        }
    }
}

impl<'tcx> fmt::Display for FrameInfo<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        ty::tls::with(|tcx| {
            if tcx.def_key(self.instance.def_id()).disambiguated_data.data
                == DefPathData::ClosureExpr
            {
                write!(f, "inside closure")?;
            } else {
                write!(f, "inside `{}`", self.instance)?;
            }
            if !self.span.is_dummy() {
                let sm = tcx.sess.source_map();
                let lo = sm.lookup_char_pos(self.span.lo());
                write!(
                    f,
                    " at {}:{}:{}",
                    sm.filename_for_diagnostics(&lo.file.name),
                    lo.line,
                    lo.col.to_usize() + 1
                )?;
            }
            Ok(())
        })
    }
}

impl<'mir, 'tcx, M: Machine<'mir, 'tcx>> HasDataLayout for InterpCx<'mir, 'tcx, M> {
    #[inline]
    fn data_layout(&self) -> &TargetDataLayout {
        &self.tcx.data_layout
    }
}

impl<'mir, 'tcx, M> layout::HasTyCtxt<'tcx> for InterpCx<'mir, 'tcx, M>
where
    M: Machine<'mir, 'tcx>,
{
    #[inline]
    fn tcx(&self) -> TyCtxt<'tcx> {
        *self.tcx
    }
}

impl<'mir, 'tcx, M> layout::HasParamEnv<'tcx> for InterpCx<'mir, 'tcx, M>
where
    M: Machine<'mir, 'tcx>,
{
    fn param_env(&self) -> ty::ParamEnv<'tcx> {
        self.param_env
    }
}

impl<'mir, 'tcx: 'mir, M: Machine<'mir, 'tcx>> LayoutOfHelpers<'tcx> for InterpCx<'mir, 'tcx, M> {
    type LayoutOfResult = InterpResult<'tcx, TyAndLayout<'tcx>>;

    #[inline]
    fn layout_tcx_at_span(&self) -> Span {
        // Using the cheap root span for performance.
        self.tcx.span
    }

    #[inline]
    fn handle_layout_err(
        &self,
        err: LayoutError<'tcx>,
        _: Span,
        _: Ty<'tcx>,
    ) -> InterpErrorInfo<'tcx> {
        err_inval!(Layout(err)).into()
    }
}

impl<'mir, 'tcx: 'mir, M: Machine<'mir, 'tcx>> FnAbiOfHelpers<'tcx> for InterpCx<'mir, 'tcx, M> {
    type FnAbiOfResult = InterpResult<'tcx, &'tcx FnAbi<'tcx, Ty<'tcx>>>;

    fn handle_fn_abi_err(
        &self,
        err: FnAbiError<'tcx>,
        _span: Span,
        _fn_abi_request: FnAbiRequest<'tcx>,
    ) -> InterpErrorInfo<'tcx> {
        match err {
            FnAbiError::Layout(err) => err_inval!(Layout(err)).into(),
            FnAbiError::AdjustForForeignAbi(err) => {
                err_inval!(FnAbiAdjustForForeignAbi(err)).into()
            }
        }
    }
}

/// Test if it is valid for a MIR assignment to assign `src`-typed place to `dest`-typed value.
/// This test should be symmetric, as it is primarily about layout compatibility.
pub(super) fn mir_assign_valid_types<'tcx>(
    tcx: TyCtxt<'tcx>,
    param_env: ParamEnv<'tcx>,
    src: TyAndLayout<'tcx>,
    dest: TyAndLayout<'tcx>,
) -> bool {
    // Type-changing assignments can happen when subtyping is used. While
    // all normal lifetimes are erased, higher-ranked types with their
    // late-bound lifetimes are still around and can lead to type
    // differences. So we compare ignoring lifetimes.
    if equal_up_to_regions(tcx, param_env, src.ty, dest.ty) {
        // Make sure the layout is equal, too -- just to be safe. Miri really
        // needs layout equality. For performance reason we skip this check when
        // the types are equal. Equal types *can* have different layouts when
        // enum downcast is involved (as enum variants carry the type of the
        // enum), but those should never occur in assignments.
        if cfg!(debug_assertions) || src.ty != dest.ty {
            assert_eq!(src.layout, dest.layout);
        }
        true
    } else {
        false
    }
}

/// Use the already known layout if given (but sanity check in debug mode),
/// or compute the layout.
#[cfg_attr(not(debug_assertions), inline(always))]
pub(super) fn from_known_layout<'tcx>(
    tcx: TyCtxtAt<'tcx>,
    param_env: ParamEnv<'tcx>,
    known_layout: Option<TyAndLayout<'tcx>>,
    compute: impl FnOnce() -> InterpResult<'tcx, TyAndLayout<'tcx>>,
) -> InterpResult<'tcx, TyAndLayout<'tcx>> {
    match known_layout {
        None => compute(),
        Some(known_layout) => {
            if cfg!(debug_assertions) {
                let check_layout = compute()?;
                if !mir_assign_valid_types(tcx.tcx, param_env, check_layout, known_layout) {
                    span_bug!(
                        tcx.span,
                        "expected type differs from actual type.\nexpected: {:?}\nactual: {:?}",
                        known_layout.ty,
                        check_layout.ty,
                    );
                }
            }
            Ok(known_layout)
        }
    }
}

impl<'mir, 'tcx: 'mir, M: Machine<'mir, 'tcx>> InterpCx<'mir, 'tcx, M> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        root_span: Span,
        param_env: ty::ParamEnv<'tcx>,
        machine: M,
    ) -> Self {
        InterpCx {
            machine,
            tcx: tcx.at(root_span),
            param_env,
            memory: Memory::new(),
            recursion_limit: tcx.recursion_limit(),
        }
    }

    #[inline(always)]
    pub fn cur_span(&self) -> Span {
        // This deliberately does *not* honor `requires_caller_location` since it is used for much
        // more than just panics.
        self.stack().last().map_or(self.tcx.span, |f| f.current_span())
    }

    #[inline(always)]
    pub(crate) fn stack(&self) -> &[Frame<'mir, 'tcx, M::Provenance, M::FrameExtra>] {
        M::stack(self)
    }

    #[inline(always)]
    pub(crate) fn stack_mut(
        &mut self,
    ) -> &mut Vec<Frame<'mir, 'tcx, M::Provenance, M::FrameExtra>> {
        M::stack_mut(self)
    }

    #[inline(always)]
    pub fn frame_idx(&self) -> usize {
        let stack = self.stack();
        assert!(!stack.is_empty());
        stack.len() - 1
    }

    #[inline(always)]
    pub fn frame(&self) -> &Frame<'mir, 'tcx, M::Provenance, M::FrameExtra> {
        self.stack().last().expect("no call frames exist")
    }

    #[inline(always)]
    pub fn frame_mut(&mut self) -> &mut Frame<'mir, 'tcx, M::Provenance, M::FrameExtra> {
        self.stack_mut().last_mut().expect("no call frames exist")
    }

    #[inline(always)]
    pub(super) fn body(&self) -> &'mir mir::Body<'tcx> {
        self.frame().body
    }

    #[inline(always)]
    pub fn sign_extend(&self, value: u128, ty: TyAndLayout<'_>) -> u128 {
        assert!(ty.abi.is_signed());
        ty.size.sign_extend(value)
    }

    #[inline(always)]
    pub fn truncate(&self, value: u128, ty: TyAndLayout<'_>) -> u128 {
        ty.size.truncate(value)
    }

    #[inline]
    pub fn type_is_freeze(&self, ty: Ty<'tcx>) -> bool {
        ty.is_freeze(self.tcx, self.param_env)
    }

    pub fn load_mir(
        &self,
        instance: ty::InstanceDef<'tcx>,
        promoted: Option<mir::Promoted>,
    ) -> InterpResult<'tcx, &'tcx mir::Body<'tcx>> {
        let def = instance.with_opt_param();
        trace!("load mir(instance={:?}, promoted={:?})", instance, promoted);
        let body = if let Some(promoted) = promoted {
            &self.tcx.promoted_mir_opt_const_arg(def)[promoted]
        } else {
            M::load_mir(self, instance)?
        };
        // do not continue if typeck errors occurred (can only occur in local crate)
        if let Some(err) = body.tainted_by_errors {
            throw_inval!(AlreadyReported(err));
        }
        Ok(body)
    }

    /// Call this on things you got out of the MIR (so it is as generic as the current
    /// stack frame), to bring it into the proper environment for this interpreter.
    pub(super) fn subst_from_current_frame_and_normalize_erasing_regions<T: TypeFoldable<'tcx>>(
        &self,
        value: T,
    ) -> Result<T, InterpError<'tcx>> {
        self.subst_from_frame_and_normalize_erasing_regions(self.frame(), value)
    }

    /// Call this on things you got out of the MIR (so it is as generic as the provided
    /// stack frame), to bring it into the proper environment for this interpreter.
    pub(super) fn subst_from_frame_and_normalize_erasing_regions<T: TypeFoldable<'tcx>>(
        &self,
        frame: &Frame<'mir, 'tcx, M::Provenance, M::FrameExtra>,
        value: T,
    ) -> Result<T, InterpError<'tcx>> {
        frame
            .instance
            .try_subst_mir_and_normalize_erasing_regions(*self.tcx, self.param_env, value)
            .map_err(|e| {
                self.tcx.sess.delay_span_bug(
                    self.cur_span(),
                    format!("failed to normalize {}", e.get_type_for_failure()).as_str(),
                );

                InterpError::InvalidProgram(InvalidProgramInfo::TooGeneric)
            })
    }

    /// The `substs` are assumed to already be in our interpreter "universe" (param_env).
    pub(super) fn resolve(
        &self,
        def: ty::WithOptConstParam<DefId>,
        substs: SubstsRef<'tcx>,
    ) -> InterpResult<'tcx, ty::Instance<'tcx>> {
        trace!("resolve: {:?}, {:#?}", def, substs);
        trace!("param_env: {:#?}", self.param_env);
        trace!("substs: {:#?}", substs);
        match ty::Instance::resolve_opt_const_arg(*self.tcx, self.param_env, def, substs) {
            Ok(Some(instance)) => Ok(instance),
            Ok(None) => throw_inval!(TooGeneric),

            // FIXME(eddyb) this could be a bit more specific than `AlreadyReported`.
            Err(error_reported) => throw_inval!(AlreadyReported(error_reported)),
        }
    }

    #[inline(always)]
    pub fn layout_of_local(
        &self,
        frame: &Frame<'mir, 'tcx, M::Provenance, M::FrameExtra>,
        local: mir::Local,
        layout: Option<TyAndLayout<'tcx>>,
    ) -> InterpResult<'tcx, TyAndLayout<'tcx>> {
        // `const_prop` runs into this with an invalid (empty) frame, so we
        // have to support that case (mostly by skipping all caching).
        match frame.locals.get(local).and_then(|state| state.layout.get()) {
            None => {
                let layout = from_known_layout(self.tcx, self.param_env, layout, || {
                    let local_ty = frame.body.local_decls[local].ty;
                    let local_ty =
                        self.subst_from_frame_and_normalize_erasing_regions(frame, local_ty)?;
                    self.layout_of(local_ty)
                })?;
                if let Some(state) = frame.locals.get(local) {
                    // Layouts of locals are requested a lot, so we cache them.
                    state.layout.set(Some(layout));
                }
                Ok(layout)
            }
            Some(layout) => Ok(layout),
        }
    }

    /// Returns the actual dynamic size and alignment of the place at the given type.
    /// Only the "meta" (metadata) part of the place matters.
    /// This can fail to provide an answer for extern types.
    pub(super) fn size_and_align_of(
        &self,
        metadata: &MemPlaceMeta<M::Provenance>,
        layout: &TyAndLayout<'tcx>,
    ) -> InterpResult<'tcx, Option<(Size, Align)>> {
        if !layout.is_unsized() {
            return Ok(Some((layout.size, layout.align.abi)));
        }
        match layout.ty.kind() {
            ty::Adt(..) | ty::Tuple(..) => {
                // First get the size of all statically known fields.
                // Don't use type_of::sizing_type_of because that expects t to be sized,
                // and it also rounds up to alignment, which we want to avoid,
                // as the unsized field's alignment could be smaller.
                assert!(!layout.ty.is_simd());
                assert!(layout.fields.count() > 0);
                trace!("DST layout: {:?}", layout);

                let sized_size = layout.fields.offset(layout.fields.count() - 1);
                let sized_align = layout.align.abi;
                trace!(
                    "DST {} statically sized prefix size: {:?} align: {:?}",
                    layout.ty,
                    sized_size,
                    sized_align
                );

                // Recurse to get the size of the dynamically sized field (must be
                // the last field).  Can't have foreign types here, how would we
                // adjust alignment and size for them?
                let field = layout.field(self, layout.fields.count() - 1);
                let Some((unsized_size, unsized_align)) = self.size_and_align_of(metadata, &field)? else {
                    // A field with an extern type. We don't know the actual dynamic size
                    // or the alignment.
                    return Ok(None);
                };

                // FIXME (#26403, #27023): We should be adding padding
                // to `sized_size` (to accommodate the `unsized_align`
                // required of the unsized field that follows) before
                // summing it with `sized_size`. (Note that since #26403
                // is unfixed, we do not yet add the necessary padding
                // here. But this is where the add would go.)

                // Return the sum of sizes and max of aligns.
                let size = sized_size + unsized_size; // `Size` addition

                // Choose max of two known alignments (combined value must
                // be aligned according to more restrictive of the two).
                let align = sized_align.max(unsized_align);

                // Issue #27023: must add any necessary padding to `size`
                // (to make it a multiple of `align`) before returning it.
                let size = size.align_to(align);

                // Check if this brought us over the size limit.
                if size > self.max_size_of_val() {
                    throw_ub!(InvalidMeta("total size is bigger than largest supported object"));
                }
                Ok(Some((size, align)))
            }
            ty::Dynamic(..) => {
                let vtable = metadata.unwrap_meta().to_pointer(self)?;
                // Read size and align from vtable (already checks size).
                Ok(Some(self.get_vtable_size_and_align(vtable)?))
            }

            ty::Slice(_) | ty::Str => {
                let len = metadata.unwrap_meta().to_machine_usize(self)?;
                let elem = layout.field(self, 0);

                // Make sure the slice is not too big.
                let size = elem.size.bytes().saturating_mul(len); // we rely on `max_size_of_val` being smaller than `u64::MAX`.
                let size = Size::from_bytes(size);
                if size > self.max_size_of_val() {
                    throw_ub!(InvalidMeta("slice is bigger than largest supported object"));
                }
                Ok(Some((size, elem.align.abi)))
            }

            ty::Foreign(_) => Ok(None),

            _ => span_bug!(self.cur_span(), "size_and_align_of::<{:?}> not supported", layout.ty),
        }
    }
    #[inline]
    pub fn size_and_align_of_mplace(
        &self,
        mplace: &MPlaceTy<'tcx, M::Provenance>,
    ) -> InterpResult<'tcx, Option<(Size, Align)>> {
        self.size_and_align_of(&mplace.meta, &mplace.layout)
    }

    #[instrument(skip(self, body, return_place, return_to_block), level = "debug")]
    pub fn push_stack_frame(
        &mut self,
        instance: ty::Instance<'tcx>,
        body: &'mir mir::Body<'tcx>,
        return_place: &PlaceTy<'tcx, M::Provenance>,
        return_to_block: StackPopCleanup,
    ) -> InterpResult<'tcx> {
        trace!("body: {:#?}", body);
        // first push a stack frame so we have access to the local substs
        let pre_frame = Frame {
            body,
            loc: Err(body.span), // Span used for errors caused during preamble.
            return_to_block,
            return_place: return_place.clone(),
            // empty local array, we fill it in below, after we are inside the stack frame and
            // all methods actually know about the frame
            locals: IndexVec::new(),
            instance,
            tracing_span: SpanGuard::new(),
            extra: (),
        };
        let frame = M::init_frame_extra(self, pre_frame)?;
        self.stack_mut().push(frame);

        // Make sure all the constants required by this frame evaluate successfully (post-monomorphization check).
        for ct in &body.required_consts {
            let span = ct.span;
            let ct = self.subst_from_current_frame_and_normalize_erasing_regions(ct.literal)?;
            self.const_to_op(&ct, None).map_err(|err| {
                // If there was an error, set the span of the current frame to this constant.
                // Avoiding doing this when evaluation succeeds.
                self.frame_mut().loc = Err(span);
                err
            })?;
        }

        // Most locals are initially dead.
        let dummy = LocalState { value: LocalValue::Dead, layout: Cell::new(None) };
        let mut locals = IndexVec::from_elem(dummy, &body.local_decls);

        // Now mark those locals as live that have no `Storage*` annotations.
        let always_live = always_storage_live_locals(self.body());
        for local in locals.indices() {
            if always_live.contains(local) {
                locals[local].value = LocalValue::Live(Operand::Immediate(Immediate::Uninit));
            }
        }
        // done
        self.frame_mut().locals = locals;
        M::after_stack_push(self)?;
        self.frame_mut().loc = Ok(mir::Location::START);

        let span = info_span!("frame", "{}", instance);
        self.frame_mut().tracing_span.enter(span);

        Ok(())
    }

    /// Jump to the given block.
    #[inline]
    pub fn go_to_block(&mut self, target: mir::BasicBlock) {
        self.frame_mut().loc = Ok(mir::Location { block: target, statement_index: 0 });
    }

    /// *Return* to the given `target` basic block.
    /// Do *not* use for unwinding! Use `unwind_to_block` instead.
    ///
    /// If `target` is `None`, that indicates the function cannot return, so we raise UB.
    pub fn return_to_block(&mut self, target: Option<mir::BasicBlock>) -> InterpResult<'tcx> {
        if let Some(target) = target {
            self.go_to_block(target);
            Ok(())
        } else {
            throw_ub!(Unreachable)
        }
    }

    /// *Unwind* to the given `target` basic block.
    /// Do *not* use for returning! Use `return_to_block` instead.
    ///
    /// If `target` is `StackPopUnwind::Skip`, that indicates the function does not need cleanup
    /// during unwinding, and we will just keep propagating that upwards.
    ///
    /// If `target` is `StackPopUnwind::NotAllowed`, that indicates the function does not allow
    /// unwinding, and doing so is UB.
    pub fn unwind_to_block(&mut self, target: StackPopUnwind) -> InterpResult<'tcx> {
        self.frame_mut().loc = match target {
            StackPopUnwind::Cleanup(block) => Ok(mir::Location { block, statement_index: 0 }),
            StackPopUnwind::Skip => Err(self.frame_mut().body.span),
            StackPopUnwind::NotAllowed => {
                throw_ub_format!("unwinding past a stack frame that does not allow unwinding")
            }
        };
        Ok(())
    }

    /// Pops the current frame from the stack, deallocating the
    /// memory for allocated locals.
    ///
    /// If `unwinding` is `false`, then we are performing a normal return
    /// from a function. In this case, we jump back into the frame of the caller,
    /// and continue execution as normal.
    ///
    /// If `unwinding` is `true`, then we are in the middle of a panic,
    /// and need to unwind this frame. In this case, we jump to the
    /// `cleanup` block for the function, which is responsible for running
    /// `Drop` impls for any locals that have been initialized at this point.
    /// The cleanup block ends with a special `Resume` terminator, which will
    /// cause us to continue unwinding.
    #[instrument(skip(self), level = "debug")]
    pub(super) fn pop_stack_frame(&mut self, unwinding: bool) -> InterpResult<'tcx> {
        info!(
            "popping stack frame ({})",
            if unwinding { "during unwinding" } else { "returning from function" }
        );

        // Check `unwinding`.
        assert_eq!(
            unwinding,
            match self.frame().loc {
                Ok(loc) => self.body().basic_blocks[loc.block].is_cleanup,
                Err(_) => true,
            }
        );
        if unwinding && self.frame_idx() == 0 {
            throw_ub_format!("unwinding past the topmost frame of the stack");
        }

        // Copy return value. Must of course happen *before* we deallocate the locals.
        let copy_ret_result = if !unwinding {
            let op = self
                .local_to_op(self.frame(), mir::RETURN_PLACE, None)
                .expect("return place should always be live");
            let dest = self.frame().return_place.clone();
            let err = self.copy_op(&op, &dest, /*allow_transmute*/ true);
            trace!("return value: {:?}", self.dump_place(*dest));
            // We delay actually short-circuiting on this error until *after* the stack frame is
            // popped, since we want this error to be attributed to the caller, whose type defines
            // this transmute.
            err
        } else {
            Ok(())
        };

        // Cleanup: deallocate locals.
        // Usually we want to clean up (deallocate locals), but in a few rare cases we don't.
        // We do this while the frame is still on the stack, so errors point to the callee.
        let return_to_block = self.frame().return_to_block;
        let cleanup = match return_to_block {
            StackPopCleanup::Goto { .. } => true,
            StackPopCleanup::Root { cleanup, .. } => cleanup,
        };
        if cleanup {
            // We need to take the locals out, since we need to mutate while iterating.
            let locals = mem::take(&mut self.frame_mut().locals);
            for local in &locals {
                self.deallocate_local(local.value)?;
            }
        }

        // All right, now it is time to actually pop the frame.
        // Note that its locals are gone already, but that's fine.
        let frame =
            self.stack_mut().pop().expect("tried to pop a stack frame, but there were none");
        // Report error from return value copy, if any.
        copy_ret_result?;

        // If we are not doing cleanup, also skip everything else.
        if !cleanup {
            assert!(self.stack().is_empty(), "only the topmost frame should ever be leaked");
            assert!(!unwinding, "tried to skip cleanup during unwinding");
            // Skip machine hook.
            return Ok(());
        }
        if M::after_stack_pop(self, frame, unwinding)? == StackPopJump::NoJump {
            // The hook already did everything.
            return Ok(());
        }

        // Normal return, figure out where to jump.
        if unwinding {
            // Follow the unwind edge.
            let unwind = match return_to_block {
                StackPopCleanup::Goto { unwind, .. } => unwind,
                StackPopCleanup::Root { .. } => {
                    panic!("encountered StackPopCleanup::Root when unwinding!")
                }
            };
            self.unwind_to_block(unwind)
        } else {
            // Follow the normal return edge.
            match return_to_block {
                StackPopCleanup::Goto { ret, .. } => self.return_to_block(ret),
                StackPopCleanup::Root { .. } => {
                    assert!(
                        self.stack().is_empty(),
                        "only the topmost frame can have StackPopCleanup::Root"
                    );
                    Ok(())
                }
            }
        }
    }

    /// Mark a storage as live, killing the previous content.
    pub fn storage_live(&mut self, local: mir::Local) -> InterpResult<'tcx> {
        assert!(local != mir::RETURN_PLACE, "Cannot make return place live");
        trace!("{:?} is now live", local);

        let local_val = LocalValue::Live(Operand::Immediate(Immediate::Uninit));
        // StorageLive expects the local to be dead, and marks it live.
        let old = mem::replace(&mut self.frame_mut().locals[local].value, local_val);
        if !matches!(old, LocalValue::Dead) {
            throw_ub_format!("StorageLive on a local that was already live");
        }
        Ok(())
    }

    pub fn storage_dead(&mut self, local: mir::Local) -> InterpResult<'tcx> {
        assert!(local != mir::RETURN_PLACE, "Cannot make return place dead");
        trace!("{:?} is now dead", local);

        // It is entirely okay for this local to be already dead (at least that's how we currently generate MIR)
        let old = mem::replace(&mut self.frame_mut().locals[local].value, LocalValue::Dead);
        self.deallocate_local(old)?;
        Ok(())
    }

    #[instrument(skip(self), level = "debug")]
    fn deallocate_local(&mut self, local: LocalValue<M::Provenance>) -> InterpResult<'tcx> {
        if let LocalValue::Live(Operand::Indirect(MemPlace { ptr, .. })) = local {
            // All locals have a backing allocation, even if the allocation is empty
            // due to the local having ZST type. Hence we can `unwrap`.
            trace!(
                "deallocating local {:?}: {:?}",
                local,
                // Locals always have a `alloc_id` (they are never the result of a int2ptr).
                self.dump_alloc(ptr.provenance.unwrap().get_alloc_id().unwrap())
            );
            self.deallocate_ptr(ptr, None, MemoryKind::Stack)?;
        };
        Ok(())
    }

    pub fn eval_to_allocation(
        &self,
        gid: GlobalId<'tcx>,
    ) -> InterpResult<'tcx, MPlaceTy<'tcx, M::Provenance>> {
        // For statics we pick `ParamEnv::reveal_all`, because statics don't have generics
        // and thus don't care about the parameter environment. While we could just use
        // `self.param_env`, that would mean we invoke the query to evaluate the static
        // with different parameter environments, thus causing the static to be evaluated
        // multiple times.
        let param_env = if self.tcx.is_static(gid.instance.def_id()) {
            ty::ParamEnv::reveal_all()
        } else {
            self.param_env
        };
        let param_env = param_env.with_const();
        // Use a precise span for better cycle errors.
        let val = self.tcx.at(self.cur_span()).eval_to_allocation_raw(param_env.and(gid))?;
        self.raw_const_to_mplace(val)
    }

    #[must_use]
    pub fn dump_place(&self, place: Place<M::Provenance>) -> PlacePrinter<'_, 'mir, 'tcx, M> {
        PlacePrinter { ecx: self, place }
    }

    #[must_use]
    pub fn generate_stacktrace_from_stack(
        stack: &[Frame<'mir, 'tcx, M::Provenance, M::FrameExtra>],
    ) -> Vec<FrameInfo<'tcx>> {
        let mut frames = Vec::new();
        // This deliberately does *not* honor `requires_caller_location` since it is used for much
        // more than just panics.
        for frame in stack.iter().rev() {
            let lint_root = frame.current_source_info().and_then(|source_info| {
                match &frame.body.source_scopes[source_info.scope].local_data {
                    mir::ClearCrossCrate::Set(data) => Some(data.lint_root),
                    mir::ClearCrossCrate::Clear => None,
                }
            });
            let span = frame.current_span();

            frames.push(FrameInfo { span, instance: frame.instance, lint_root });
        }
        trace!("generate stacktrace: {:#?}", frames);
        frames
    }

    #[must_use]
    pub fn generate_stacktrace(&self) -> Vec<FrameInfo<'tcx>> {
        Self::generate_stacktrace_from_stack(self.stack())
    }
}

#[doc(hidden)]
/// Helper struct for the `dump_place` function.
pub struct PlacePrinter<'a, 'mir, 'tcx, M: Machine<'mir, 'tcx>> {
    ecx: &'a InterpCx<'mir, 'tcx, M>,
    place: Place<M::Provenance>,
}

impl<'a, 'mir, 'tcx: 'mir, M: Machine<'mir, 'tcx>> std::fmt::Debug
    for PlacePrinter<'a, 'mir, 'tcx, M>
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.place {
            Place::Local { frame, local } => {
                let mut allocs = Vec::new();
                write!(fmt, "{:?}", local)?;
                if frame != self.ecx.frame_idx() {
                    write!(fmt, " ({} frames up)", self.ecx.frame_idx() - frame)?;
                }
                write!(fmt, ":")?;

                match self.ecx.stack()[frame].locals[local].value {
                    LocalValue::Dead => write!(fmt, " is dead")?,
                    LocalValue::Live(Operand::Immediate(Immediate::Uninit)) => {
                        write!(fmt, " is uninitialized")?
                    }
                    LocalValue::Live(Operand::Indirect(mplace)) => {
                        write!(
                            fmt,
                            " by {} ref {:?}:",
                            match mplace.meta {
                                MemPlaceMeta::Meta(meta) => format!(" meta({:?})", meta),
                                MemPlaceMeta::None => String::new(),
                            },
                            mplace.ptr,
                        )?;
                        allocs.extend(mplace.ptr.provenance.map(Provenance::get_alloc_id));
                    }
                    LocalValue::Live(Operand::Immediate(Immediate::Scalar(val))) => {
                        write!(fmt, " {:?}", val)?;
                        if let Scalar::Ptr(ptr, _size) = val {
                            allocs.push(ptr.provenance.get_alloc_id());
                        }
                    }
                    LocalValue::Live(Operand::Immediate(Immediate::ScalarPair(val1, val2))) => {
                        write!(fmt, " ({:?}, {:?})", val1, val2)?;
                        if let Scalar::Ptr(ptr, _size) = val1 {
                            allocs.push(ptr.provenance.get_alloc_id());
                        }
                        if let Scalar::Ptr(ptr, _size) = val2 {
                            allocs.push(ptr.provenance.get_alloc_id());
                        }
                    }
                }

                write!(fmt, ": {:?}", self.ecx.dump_allocs(allocs.into_iter().flatten().collect()))
            }
            Place::Ptr(mplace) => match mplace.ptr.provenance.and_then(Provenance::get_alloc_id) {
                Some(alloc_id) => {
                    write!(fmt, "by ref {:?}: {:?}", mplace.ptr, self.ecx.dump_alloc(alloc_id))
                }
                ptr => write!(fmt, " integral by ref: {:?}", ptr),
            },
        }
    }
}
