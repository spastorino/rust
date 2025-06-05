use itertools::{Either, Itertools};
use rustc_data_structures::fx::{FxHashSet, FxIndexSet};
use rustc_middle::mir::visit::{TyContext, Visitor};
use rustc_middle::mir::{Body, Local, Location, SourceInfo};
use rustc_middle::span_bug;
use rustc_middle::ty::relate::Relate;
use rustc_middle::ty::{GenericArgsRef, Region, RegionVid, Ty, TyCtxt, TypeVisitable};
use rustc_mir_dataflow::move_paths::MoveData;
use rustc_mir_dataflow::points::DenseLocationMap;
use tracing::debug;

use super::TypeChecker;
use crate::constraints::OutlivesConstraintSet;
use crate::polonius::PoloniusLivenessContext;
use crate::region_infer::values::LivenessValues;
use crate::type_check::liveness::local_use_map::LocalUseMap;
use crate::universal_regions::UniversalRegions;

mod local_use_map;
mod trace;

/// Combines liveness analysis with initialization analysis to
/// determine which variables are live at which points, both due to
/// ordinary uses and drops. Returns a set of (ty, location) pairs
/// that indicate which types must be live at which point in the CFG.
/// This vector is consumed by `constraint_generation`.
///
/// N.B., this computation requires normalization; therefore, it must be
/// performed before
pub(super) fn generate<'tcx>(
    typeck: &mut TypeChecker<'_, 'tcx>,
    location_map: &DenseLocationMap,
    move_data: &MoveData<'tcx>,
) {
    debug!("liveness::generate");

    let mut free_regions = regions_that_outlive_free_regions(
        typeck.infcx.num_region_vars(),
        &typeck.universal_regions,
        &typeck.constraints.outlives_constraints,
    );

    // NLLs can avoid computing some liveness data here because its constraints are
    // location-insensitive, but that doesn't work in polonius: locals whose type contains a region
    // that outlives a free region are not necessarily live everywhere in a flow-sensitive setting,
    // unlike NLLs.
    // We do record these regions in the polonius context, since they're used to differentiate
    // relevant and boring locals, which is a key distinction used later in diagnostics.
    if typeck.tcx().sess.opts.unstable_opts.polonius.is_next_enabled() {
        let (_, boring_locals) =
            compute_relevant_live_locals(typeck.tcx(), &free_regions, typeck.body);
        typeck.polonius_liveness.as_mut().unwrap().boring_nll_locals =
            boring_locals.into_iter().collect();
        free_regions = typeck.universal_regions.universal_regions_iter().collect();
    }
    let (relevant_live_locals, boring_locals) =
        compute_relevant_live_locals(typeck.tcx(), &free_regions, typeck.body);

    trace::trace(typeck, location_map, move_data, relevant_live_locals, boring_locals);

    // Mark regions that should be live where they appear within rvalues or within a call: like
    // args, regions, and types.
    record_regular_live_regions(
        typeck.tcx(),
        &mut typeck.constraints.liveness_constraints,
        &typeck.universal_regions,
        &mut typeck.polonius_liveness,
        typeck.body,
    );

    if typeck.tcx().features().ergonomic_clones() {
        let v = compute_last_use_for_all_locals(typeck, location_map);
        debug!("compute_last_use_for_all_locals = {:?}", v);
        typeck.last_uses = v;
    }
}

fn compute_last_use_for_all_locals<'tcx>(
    typeck: &mut TypeChecker<'_, 'tcx>,
    location_map: &DenseLocationMap,
) -> Vec<Location> {
    let all_locals: Vec<_> =
        typeck.body.local_decls.iter_enumerated().map(|(local, _)| local).collect();
    let local_use_map = &LocalUseMap::build(&all_locals, location_map, typeck.body);
    all_locals
        .iter()
        .flat_map(|local| compute_last_use_for(typeck, location_map, local_use_map, *local))
        .collect()
}

/// Computes all points where local is "use live" -- meaning its
/// current value may be used later (except by a drop). This is
/// done by walking backwards from each use of `local` until we
/// find a `def` of local.
///
/// Requires `add_defs_for(local)` to have been executed.
fn compute_last_use_for<'tcx>(
    typeck: &mut TypeChecker<'_, 'tcx>,
    location_map: &DenseLocationMap,
    local_use_map: &LocalUseMap,
    local: Local,
) -> Vec<Location> {
    let body = typeck.body;
    let liveness_constraints = &typeck.constraints.liveness_constraints;

    debug!("compute_last_use_for START");
    debug!("compute_last_use_for(local={:?})", local);

    // Definitions and uses must be ordered from last to first.
    let local_defs: Vec<_> = local_use_map.defs(local).collect();
    debug!("compute_last_use_for(local_defs={:?})", local_defs);
    let local_uses: Vec<_> = local_use_map.uses(local).collect();
    debug!("compute_last_use_for(local_uses={:?})", local_uses);
    let mut visited = FxIndexSet::default();

    for local_use in local_uses.iter() {
        debug!("compute_last_use_for(local_use={:?})", local_use);
        // Skip definitions, are the definitions here?
        if local_defs.iter().find(|def| **def == *local_use).is_some() {
            debug!("compute_last_use_for: skip local_use");
            continue;
        }

        let mut stack = Vec::new();
        stack.push(*local_use);

        while let Some(p) = stack.pop() {
            debug!("compute_last_use_for(p={:?})", p);
            let block_start = location_map.to_block_start(p);
            debug!("compute_last_use_for(block_start={:?})", block_start);
            // This was originally block_start..=p but I want to skip p to avoid always getting
            // the same use.
            let start_to_use = block_start..p;
            debug!("compute_last_use_for(start_to_use={:?})", start_to_use);
            let previous_def = local_defs.iter().find(|def| start_to_use.contains(*def));
            debug!("compute_last_use_for(previous_def={:?})", previous_def);
            let previous_use = local_uses.iter().find(|use_| start_to_use.contains(*use_));
            debug!("compute_last_use_for(previous_use={:?})", previous_use);

            // Is there a use before a definition? if there isn't break out of the loop and
            // continue with the next local_use
            if let Some(def) = previous_def {
                if let Some(use_) = previous_use {
                    if *def >= *use_ {
                        debug!("compute_last_use_for: skip found definition before use");
                        break;
                    }
                } else {
                    debug!("compute_last_use_for: skip found definition and no use");
                    break;
                }
            }

            if let Some(use_) = previous_use {
                debug!("compute_last_use_for: visit use={:?}", use_);
                if visited.insert(*use_) {
                    debug!("compute_last_use_for: stack.push({:?})", use_);
                    stack.push(*use_);
                } else {
                    debug!("compute_last_use_for: {:?} already in stack", use_);
                }
            } else {
                let block = location_map.to_location(block_start).block;
                debug!("compute_last_use_for(block={:?})", block);
                let predecessors: Vec<_> = body.basic_blocks.predecessors()[block]
                    .iter()
                    .map(|&pred_bb| body.terminator_loc(pred_bb))
                    .map(|pred_loc| location_map.point_from_location(pred_loc))
                    .collect();
                debug!("compute_last_use_for(predecessors={:?})", predecessors);
                for p in &predecessors {
                    debug!("compute_last_use_for: visit p={:?}", *p);
                    if visited.insert(*p) {
                        debug!("compute_last_use_for: stack.push({:?})", *p);
                        stack.push(*p);
                    }
                }
            }
        }
    }

    // Step 1. Get all the borrows that are assigned to the given local
    // For example: if we are looking for last uses of _3 in the following example
    // _3 = &1
    // Terminator::clone(_3)
    // ...
    //_10 = &1
    // ...
    //
    // We get _1 here.
    debug!("compute_last_use_for(local_uses={:?})", local_uses);
    debug!("compute_last_use_for(local={:?})", local);
    debug!("compute_last_use_for(visited={:?})", visited);
    let borrowed_locals = typeck
        .borrow_set
        .location_map()
        .iter()
        .filter_map(|(_, borrow_data)| {
            if borrow_data.assigned_place.local == local {
                Some(borrow_data.borrowed_place.local)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // Step 2. Get all the regions for the borrowed local captured in the previous step.
    // In the previous code
    // _3 = &1
    // Terminator::clone(_3)
    // ...
    //_10 = &1
    // ...
    //
    // We get both regions, the one originating at _3 = &1 and _10 = &1
    debug!("compute_last_use_for(borrowed_locals={:?})", borrowed_locals);
    let borrowed_regions = typeck.borrow_set.location_map().iter().filter_map(|(_, borrow_data)| {
        debug!("compute_last_use_for(borrow_data.borrowed_place.local={:?})", borrow_data.borrowed_place.local);
        if borrowed_locals.contains(&borrow_data.borrowed_place.local) {
            debug!("compute_last_use_for(borrow_data.borrowed_place.local={:?} mapped to region={:?})", borrow_data.borrowed_place.local, borrow_data.region);
            Some(borrow_data.region)
        } else {
            debug!("compute_last_use_for(borrow_data.borrowed_place.local={:?} not mapped)", borrow_data.borrowed_place.local);
            None
        }
    }).collect::<Vec<_>>();

    // This just prints things for debugging purposes.
    debug!(
        "compute_last_use_for(typeck.borrow_set.location_map={:?}",
        typeck.borrow_set.location_map()
    );
    debug!("compute_last_use_for(typeck.borrow_set.local_map={:?}", typeck.borrow_set.local_map());
    debug!(
        "compute_last_use_for(liveness_constraints.points().rows()={:?}",
        liveness_constraints.points().rows().collect::<Vec<_>>()
    );
    for region_vid in &borrowed_regions {
        debug!("compute_last_use_for(points for region={:?}", region_vid);
        if let Some(points) = liveness_constraints.points().row(*region_vid) {
            for point in points.iter() {
                debug!("compute_last_use_for(point={:?}", point);
            }
        }
    }

    let result = local_uses
        .iter()
        .cloned()
        .filter(|local_use| {
            !visited.contains(local_use)
                // Step 3. Given the previously calculated regions we get from liveness_constraints
                // the live points for those regions and just check if the region is live after the
                // point where the local_use was found. If so, we can't move.
                //
                // In the previous code
                // _3 = &1
                // Terminator::clone(_3)
                // ...
                //_10 = &1
                // ...
                //
                // Last use is in line Terminator::clone(_3) location but through borrows we find
                // location _10 = &1, which is last then we can't move.
                && borrowed_regions
                    .iter()
                    .find(|region_vid| {
                        liveness_constraints
                            .points()
                            .row(**region_vid)
                            .map(|interval| {
                                interval.iter().find(|point| *point >= *local_use).is_some()
                            })
                            .unwrap_or(false)
                    })
                    .is_none()
                // Step 4. Given the previously calculated regions we get from liveness_constraints
                // the live points for those regions and just check if the region is live after the
                // point where the local_use was found. If so, we can't move.
                //
                // In the previous code
                // _3 = &1
                // Terminator::clone(_3)
                // ...
                //
                // Last use is in line Terminator::clone(_3) location but through borrows we find
                // location _10 = &1, which is last then we can't move.
                && borrowed_locals.iter().find(|borrowed_local| {
                    local_use_map.uses(**borrowed_local).max().map(|borrowed_local| borrowed_local >= *local_use).unwrap_or(false)
                }).is_none()
        })
        .map(|point_index| location_map.to_location(point_index))
        .collect();

    debug!("compute_last_use_for(result={:?})", result);
    debug!("compute_last_use_for END");
    result
}

// The purpose of `compute_relevant_live_locals` is to define the subset of `Local`
// variables for which we need to do a liveness computation. We only need
// to compute whether a variable `X` is live if that variable contains
// some region `R` in its type where `R` is not known to outlive a free
// region (i.e., where `R` may be valid for just a subset of the fn body).
fn compute_relevant_live_locals<'tcx>(
    tcx: TyCtxt<'tcx>,
    free_regions: &FxHashSet<RegionVid>,
    body: &Body<'tcx>,
) -> (Vec<Local>, Vec<Local>) {
    let (boring_locals, relevant_live_locals): (Vec<_>, Vec<_>) =
        body.local_decls.iter_enumerated().partition_map(|(local, local_decl)| {
            if tcx.all_free_regions_meet(&local_decl.ty, |r| free_regions.contains(&r.as_var())) {
                Either::Left(local)
            } else {
                Either::Right(local)
            }
        });

    debug!("{} total variables", body.local_decls.len());
    debug!("{} variables need liveness", relevant_live_locals.len());
    debug!("{} regions outlive free regions", free_regions.len());

    (relevant_live_locals, boring_locals)
}

/// Computes all regions that are (currently) known to outlive free
/// regions. For these regions, we do not need to compute
/// liveness, since the outlives constraints will ensure that they
/// are live over the whole fn body anyhow.
fn regions_that_outlive_free_regions<'tcx>(
    num_region_vars: usize,
    universal_regions: &UniversalRegions<'tcx>,
    constraint_set: &OutlivesConstraintSet<'tcx>,
) -> FxHashSet<RegionVid> {
    // Build a graph of the outlives constraints thus far. This is
    // a reverse graph, so for each constraint `R1: R2` we have an
    // edge `R2 -> R1`. Therefore, if we find all regions
    // reachable from each free region, we will have all the
    // regions that are forced to outlive some free region.
    let rev_constraint_graph = constraint_set.reverse_graph(num_region_vars);
    let fr_static = universal_regions.fr_static;
    let rev_region_graph = rev_constraint_graph.region_graph(constraint_set, fr_static);

    // Stack for the depth-first search. Start out with all the free regions.
    let mut stack: Vec<_> = universal_regions.universal_regions_iter().collect();

    // Set of all free regions, plus anything that outlives them. Initially
    // just contains the free regions.
    let mut outlives_free_region: FxHashSet<_> = stack.iter().cloned().collect();

    // Do the DFS -- for each thing in the stack, find all things
    // that outlive it and add them to the set. If they are not,
    // push them onto the stack for later.
    while let Some(sub_region) = stack.pop() {
        stack.extend(
            rev_region_graph
                .outgoing_regions(sub_region)
                .filter(|&r| outlives_free_region.insert(r)),
        );
    }

    // Return the final set of things we visited.
    outlives_free_region
}

/// Some variables are "regular live" at `location` -- i.e., they may be used later. This means that
/// all regions appearing in their type must be live at `location`.
fn record_regular_live_regions<'tcx>(
    tcx: TyCtxt<'tcx>,
    liveness_constraints: &mut LivenessValues,
    universal_regions: &UniversalRegions<'tcx>,
    polonius_liveness: &mut Option<PoloniusLivenessContext>,
    body: &Body<'tcx>,
) {
    let mut visitor =
        LiveVariablesVisitor { tcx, liveness_constraints, universal_regions, polonius_liveness };
    for (bb, data) in body.basic_blocks.iter_enumerated() {
        visitor.visit_basic_block_data(bb, data);
    }
}

/// Visitor looking for regions that should be live within rvalues or calls.
struct LiveVariablesVisitor<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    liveness_constraints: &'a mut LivenessValues,
    universal_regions: &'a UniversalRegions<'tcx>,
    polonius_liveness: &'a mut Option<PoloniusLivenessContext>,
}

impl<'a, 'tcx> Visitor<'tcx> for LiveVariablesVisitor<'a, 'tcx> {
    /// We sometimes have `args` within an rvalue, or within a
    /// call. Make them live at the location where they appear.
    fn visit_args(&mut self, args: &GenericArgsRef<'tcx>, location: Location) {
        self.record_regions_live_at(*args, location);
        self.super_args(args);
    }

    /// We sometimes have `region`s within an rvalue, or within a
    /// call. Make them live at the location where they appear.
    fn visit_region(&mut self, region: Region<'tcx>, location: Location) {
        self.record_regions_live_at(region, location);
        self.super_region(region);
    }

    /// We sometimes have `ty`s within an rvalue, or within a
    /// call. Make them live at the location where they appear.
    fn visit_ty(&mut self, ty: Ty<'tcx>, ty_context: TyContext) {
        match ty_context {
            TyContext::ReturnTy(SourceInfo { span, .. })
            | TyContext::YieldTy(SourceInfo { span, .. })
            | TyContext::ResumeTy(SourceInfo { span, .. })
            | TyContext::UserTy(span)
            | TyContext::LocalDecl { source_info: SourceInfo { span, .. }, .. } => {
                span_bug!(span, "should not be visiting outside of the CFG: {:?}", ty_context);
            }
            TyContext::Location(location) => {
                self.record_regions_live_at(ty, location);
            }
        }

        self.super_ty(ty);
    }
}

impl<'a, 'tcx> LiveVariablesVisitor<'a, 'tcx> {
    /// Some variable is "regular live" at `location` -- i.e., it may be used later. This means that
    /// all regions appearing in the type of `value` must be live at `location`.
    fn record_regions_live_at<T>(&mut self, value: T, location: Location)
    where
        T: TypeVisitable<TyCtxt<'tcx>> + Relate<TyCtxt<'tcx>>,
    {
        debug!("record_regions_live_at(value={:?}, location={:?})", value, location);
        self.tcx.for_each_free_region(&value, |live_region| {
            let live_region_vid = live_region.as_var();
            self.liveness_constraints.add_location(live_region_vid, location);
        });

        // When using `-Zpolonius=next`, we record the variance of each live region.
        if let Some(polonius_liveness) = self.polonius_liveness {
            polonius_liveness.record_live_region_variance(self.tcx, self.universal_regions, value);
        }
    }
}
