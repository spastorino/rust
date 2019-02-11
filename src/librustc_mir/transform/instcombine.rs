//! Performs various peephole optimizations.

use rustc::mir::{Constant, Location, Place, Mir, Operand, ProjectionElem, Rvalue, Local};
use rustc::mir::visit::{MutVisitor, Visitor};
use rustc::ty::{TyCtxt, TyKind};
use rustc::util::nodemap::{FxHashMap, FxHashSet};
use rustc_data_structures::indexed_vec::Idx;
use std::mem;
use crate::transform::{MirPass, MirSource};

pub struct InstCombine;

impl MirPass for InstCombine {
    fn run_pass<'a, 'tcx>(&self,
                          tcx: TyCtxt<'a, 'tcx, 'tcx>,
                          _: MirSource<'tcx>,
                          mir: &mut Mir<'tcx>) {
        // We only run when optimizing MIR (at any level).
        if tcx.sess.opts.debugging_opts.mir_opt_level == 0 {
            return
        }

        // First, find optimization opportunities. This is done in a pre-pass to keep the MIR
        // read-only so that we can do global analyses on the MIR in the process (e.g.
        // `Place::ty()`).
        let optimizations = {
            let mut optimization_finder = OptimizationFinder::new(mir, tcx);
            optimization_finder.visit_mir(mir);
            optimization_finder.optimizations
        };

        // Then carry out those optimizations.
        MutVisitor::visit_mir(&mut InstCombineVisitor { optimizations, tcx }, mir);
    }
}

pub struct InstCombineVisitor<'a, 'tcx: 'a> {
    optimizations: OptimizationList<'tcx>,
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
}

impl<'a, 'tcx> MutVisitor<'a, 'tcx, 'tcx> for InstCombineVisitor<'a, 'tcx> {
    fn tcx(&self) -> TyCtxt<'a, 'tcx, 'tcx> {
        self.tcx
    }

    fn visit_rvalue(&mut self, rvalue: &mut Rvalue<'tcx>, location: Location) {
        if self.optimizations.and_stars.remove(&location) {
            debug!("Replacing `&*`: {:?}", rvalue);
            let new_place = match *rvalue {
                Rvalue::Ref(_, _, Place::Projection(ref mut projection)) => {
                    // Replace with dummy
                    mem::replace(&mut projection.base, Place::Local(Local::new(0)))
                }
                _ => bug!("Detected `&*` but didn't find `&*`!"),
            };
            *rvalue = Rvalue::Use(Operand::Copy(new_place))
        }

        if let Some(constant) = self.optimizations.arrays_lengths.remove(&location) {
            debug!("Replacing `Len([_; N])`: {:?}", rvalue);
            *rvalue = Rvalue::Use(Operand::Constant(box constant));
        }

        self.super_rvalue(rvalue, location)
    }
}

/// Finds optimization opportunities on the MIR.
struct OptimizationFinder<'b, 'a, 'tcx:'a+'b> {
    mir: &'b Mir<'tcx>,
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
    optimizations: OptimizationList<'tcx>,
}

impl<'b, 'a, 'tcx:'b> OptimizationFinder<'b, 'a, 'tcx> {
    fn new(mir: &'b Mir<'tcx>, tcx: TyCtxt<'a, 'tcx, 'tcx>) -> OptimizationFinder<'b, 'a, 'tcx> {
        OptimizationFinder {
            mir,
            tcx,
            optimizations: OptimizationList::default(),
        }
    }
}

impl<'b, 'a, 'tcx> Visitor<'tcx> for OptimizationFinder<'b, 'a, 'tcx> {
    fn visit_rvalue(&mut self, rvalue: &Rvalue<'tcx>, location: Location) {
        if let Rvalue::Ref(_, _, Place::Projection(ref projection)) = *rvalue {
            if let ProjectionElem::Deref = projection.elem {
                let neo_base = self.tcx.as_new_place(&projection.base);
                if neo_base.ty(self.mir, self.tcx).to_ty(self.tcx).is_region_ptr() {
                    self.optimizations.and_stars.insert(location);
                }
            }
        }

        if let Rvalue::Len(ref place) = *rvalue {
            let neo_place = self.tcx.as_new_place(place);
            let place_ty = neo_place.ty(&self.mir.local_decls, self.tcx).to_ty(self.tcx);
            if let TyKind::Array(_, len) = place_ty.sty {
                let span = self.mir.source_info(location).span;
                let ty = self.tcx.types.usize;
                let constant = Constant { span, ty, literal: len, user_ty: None };
                self.optimizations.arrays_lengths.insert(location, constant);
            }
        }

        self.super_rvalue(rvalue, location)
    }
}

#[derive(Default)]
struct OptimizationList<'tcx> {
    and_stars: FxHashSet<Location>,
    arrays_lengths: FxHashMap<Location, Constant<'tcx>>,
}
