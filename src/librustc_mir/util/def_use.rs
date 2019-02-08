//! Def-use analysis.

use rustc::mir::{Local, Location, Mir};
use rustc::mir::visit::{PlaceContext, MutVisitor, Visitor};
use rustc::ty::TyCtxt;
use rustc_data_structures::indexed_vec::IndexVec;
use std::marker::PhantomData;
use std::mem;
use std::slice;
use std::iter;

pub struct DefUseAnalysis<'tcx> {
    info: IndexVec<Local, Info<'tcx>>,
}

#[derive(Clone)]
pub struct Info<'tcx> {
    pub defs_and_uses: Vec<Use<'tcx>>,
}

#[derive(Clone)]
pub struct Use<'tcx> {
    pub context: PlaceContext<'tcx>,
    pub location: Location,
}

impl<'tcx> DefUseAnalysis<'tcx> {
    pub fn new(mir: &Mir<'tcx>) -> DefUseAnalysis<'tcx> {
        DefUseAnalysis {
            info: IndexVec::from_elem_n(Info::new(), mir.local_decls.len()),
        }
    }

    pub fn analyze(&mut self, mir: &Mir<'tcx>) {
        self.clear();

        let mut finder = DefUseFinder {
            info: mem::replace(&mut self.info, IndexVec::new()),
        };
        finder.visit_mir(mir);
        self.info = finder.info
    }

    fn clear(&mut self) {
        for info in &mut self.info {
            info.clear();
        }
    }

    pub fn local_info(&self, local: Local) -> &Info<'tcx> {
        &self.info[local]
    }

    fn mutate_defs_and_uses<F>(&self,
                               local: Local,
                               mir: &mut Mir<'tcx>,
                               tcx: TyCtxt<'a, 'tcx, 'tcx>,
                               mut callback: F)
        where F: for<'b> FnMut(&'b mut Local,
              PlaceContext<'tcx>,
              Location) {
            for place_use in &self.info[local].defs_and_uses {
                MutateUseVisitor::new(tcx,
                                      local,
                                      &mut callback,
                                      mir).visit_location(mir, place_use.location)
            }
        }

    /// FIXME(pcwalton): This should update the def-use chains.
    pub fn replace_all_defs_and_uses_with(&self,
                                          local: Local,
                                          mir: &mut Mir<'tcx>,
                                          tcx: TyCtxt<'a, 'tcx, 'tcx>,
                                          new_local: Local) {
        self.mutate_defs_and_uses(local, mir, tcx, |local, _, _| *local = new_local)
    }
}

struct DefUseFinder<'tcx> {
    info: IndexVec<Local, Info<'tcx>>,
}

impl<'tcx> Visitor<'tcx> for DefUseFinder<'tcx> {
    fn visit_local(&mut self,
                   &local: &Local,
                   context: PlaceContext<'tcx>,
                   location: Location) {
        self.info[local].defs_and_uses.push(Use {
            context,
            location,
        });
    }
}

impl<'tcx> Info<'tcx> {
    fn new() -> Info<'tcx> {
        Info {
            defs_and_uses: vec![],
        }
    }

    fn clear(&mut self) {
        self.defs_and_uses.clear();
    }

    pub fn def_count(&self) -> usize {
        self.defs_and_uses.iter().filter(|place_use| place_use.context.is_mutating_use()).count()
    }

    pub fn def_count_not_including_drop(&self) -> usize {
        self.defs_not_including_drop().count()
    }

    pub fn defs_not_including_drop(
        &self,
    ) -> iter::Filter<slice::Iter<'_, Use<'tcx>>, fn(&&Use<'tcx>) -> bool> {
        self.defs_and_uses.iter().filter(|place_use| {
            place_use.context.is_mutating_use() && !place_use.context.is_drop()
        })
    }

    pub fn use_count(&self) -> usize {
        self.defs_and_uses.iter().filter(|place_use| {
            place_use.context.is_nonmutating_use()
        }).count()
    }
}

struct MutateUseVisitor<'a, 'tcx: 'a, F> {
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
    query: Local,
    callback: F,
    phantom: PhantomData<&'tcx ()>,
}

impl<'a, 'tcx, F> MutateUseVisitor<'a, 'tcx, F> {
    fn new(tcx: TyCtxt<'a, 'tcx, 'tcx>, query: Local, callback: F, _: &Mir<'tcx>)
           -> MutateUseVisitor<'a, 'tcx, F>
           where F: for<'b> FnMut(&'b mut Local, PlaceContext<'tcx>, Location) {
        MutateUseVisitor {
            tcx,
            query,
            callback,
            phantom: PhantomData,
        }
    }
}

impl<'a, 'tcx, F> MutVisitor<'a, 'tcx, 'tcx> for MutateUseVisitor<'a, 'tcx, F>
              where F: for<'b> FnMut(&'b mut Local, PlaceContext<'tcx>, Location) {
    fn tcx(&self) -> TyCtxt<'a, 'tcx, 'tcx> {
        self.tcx
    }

    fn visit_local(&mut self,
                    local: &mut Local,
                    context: PlaceContext<'tcx>,
                    location: Location) {
        if *local == self.query {
            (self.callback)(local, context, location)
        }
    }
}
