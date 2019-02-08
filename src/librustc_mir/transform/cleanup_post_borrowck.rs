//! This module provides two passes:
//!
//!   - [`CleanAscribeUserType`], that replaces all [`AscribeUserType`]
//!     statements with [`Nop`].
//!   - [`CleanFakeReadsAndBorrows`], that replaces all [`FakeRead`] statements
//!     and borrows that are read by [`ForMatchGuard`] fake reads with [`Nop`].
//!
//! The `CleanFakeReadsAndBorrows` "pass" is actually implemented as two
//! traversals (aka visits) of the input MIR. The first traversal,
//! [`DeleteAndRecordFakeReads`], deletes the fake reads and finds the
//! temporaries read by [`ForMatchGuard`] reads, and [`DeleteFakeBorrows`]
//! deletes the initialization of those temporaries.
//!
//! [`CleanAscribeUserType`]: cleanup_post_borrowck::CleanAscribeUserType
//! [`CleanFakeReadsAndBorrows`]: cleanup_post_borrowck::CleanFakeReadsAndBorrows
//! [`DeleteAndRecordFakeReads`]: cleanup_post_borrowck::DeleteAndRecordFakeReads
//! [`DeleteFakeBorrows`]: cleanup_post_borrowck::DeleteFakeBorrows
//! [`AscribeUserType`]: rustc::mir::StatementKind::AscribeUserType
//! [`Nop`]: rustc::mir::StatementKind::Nop
//! [`FakeRead`]: rustc::mir::StatementKind::FakeRead
//! [`ForMatchGuard`]: rustc::mir::FakeReadCause::ForMatchGuard

use rustc_data_structures::fx::FxHashSet;

use rustc::mir::{BasicBlock, FakeReadCause, Local, Location, Mir, Place, NeoPlace, PlaceBase};
use rustc::mir::{Statement, StatementKind};
use rustc::mir::visit::MutVisitor;
use rustc::ty::TyCtxt;
use crate::transform::{MirPass, MirSource};

pub struct CleanAscribeUserType;

pub struct DeleteAscribeUserType<'a, 'tcx: 'a> {
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
}

impl MirPass for CleanAscribeUserType {
    fn run_pass<'a, 'tcx>(&self,
                          tcx: TyCtxt<'a, 'tcx, 'tcx>,
                          _source: MirSource<'tcx>,
                          mir: &mut Mir<'tcx>) {
        let mut delete = DeleteAscribeUserType { tcx };
        delete.visit_mir(mir);
    }
}

impl<'a, 'tcx> MutVisitor<'a, 'tcx, 'tcx> for DeleteAscribeUserType<'a, 'tcx> {
    fn tcx(&self) -> TyCtxt<'a, 'tcx, 'tcx> {
        self.tcx
    }

    fn visit_statement(&mut self,
                       block: BasicBlock,
                       statement: &mut Statement<'tcx>,
                       location: Location) {
        if let StatementKind::AscribeUserType(..) = statement.kind {
            statement.make_nop();
        }
        self.super_statement(block, statement, location);
    }
}

pub struct CleanFakeReadsAndBorrows;

pub struct DeleteAndRecordFakeReads<'a, 'tcx: 'a> {
    fake_borrow_temporaries: FxHashSet<Local>,
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
}

pub struct DeleteFakeBorrows<'a, 'tcx: 'a> {
    fake_borrow_temporaries: FxHashSet<Local>,
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
}

// Removes any FakeReads from the MIR
impl MirPass for CleanFakeReadsAndBorrows {
    fn run_pass<'a, 'tcx>(&self,
                          tcx: TyCtxt<'a, 'tcx, 'tcx>,
                          _source: MirSource<'tcx>,
                          mir: &mut Mir<'tcx>) {
        let mut delete_reads = DeleteAndRecordFakeReads {
            fake_borrow_temporaries: FxHashSet::default(),
            tcx,
        };
        delete_reads.visit_mir(mir);
        let mut delete_borrows = DeleteFakeBorrows {
            fake_borrow_temporaries: delete_reads.fake_borrow_temporaries,
            tcx,
        };
        delete_borrows.visit_mir(mir);
    }
}

impl<'a, 'tcx> MutVisitor<'a, 'tcx, 'tcx> for DeleteAndRecordFakeReads<'a, 'tcx> {
    fn tcx(&self) -> TyCtxt<'a, 'tcx, 'tcx> {
        self.tcx
    }

    fn visit_statement(&mut self,
                       block: BasicBlock,
                       statement: &mut Statement<'tcx>,
                       location: Location) {
        if let StatementKind::FakeRead(cause, ref place) = statement.kind {
            if let FakeReadCause::ForMatchGuard = cause {
                match *place {
                    Place::Local(local) => self.fake_borrow_temporaries.insert(local),
                    _ => bug!("Fake match guard read of non-local: {:?}", place),
                };
            }
            statement.make_nop();
        }
        self.super_statement(block, statement, location);
    }
}

impl<'a, 'tcx> MutVisitor<'a, 'tcx, 'tcx> for DeleteFakeBorrows<'a, 'tcx> {
    fn tcx(&self) -> TyCtxt<'a, 'tcx, 'tcx> {
        self.tcx
    }

    fn visit_statement(&mut self,
                       block: BasicBlock,
                       statement: &mut Statement<'tcx>,
                       location: Location) {
        if let StatementKind::Assign(NeoPlace {
            base: PlaceBase::Local(local),
            elems: &[],
        }, _) = statement.kind {
            if self.fake_borrow_temporaries.contains(&local) {
                statement.make_nop();
            }
        }
        self.super_statement(block, statement, location);
    }
}
