//! Analysis of patterns, notably match exhaustiveness checking.

pub mod constructor;
#[cfg(feature = "rustc")]
pub mod errors;
#[cfg(feature = "rustc")]
pub(crate) mod lints;
pub mod pat;
#[cfg(feature = "rustc")]
pub mod rustc;
pub mod usefulness;

#[macro_use]
extern crate tracing;
#[cfg(feature = "rustc")]
#[macro_use]
extern crate rustc_middle;

#[cfg(feature = "rustc")]
rustc_fluent_macro::fluent_messages! { "../messages.ftl" }

use std::fmt;

use rustc_index::Idx;
#[cfg(feature = "rustc")]
use rustc_middle::ty::Ty;

use crate::constructor::{Constructor, ConstructorSet};
#[cfg(feature = "rustc")]
use crate::lints::{
    lint_nonexhaustive_missing_variants, lint_overlapping_range_endpoints, PatternColumn,
};
use crate::pat::DeconstructedPat;
#[cfg(feature = "rustc")]
use crate::rustc::RustcMatchCheckCtxt;
#[cfg(feature = "rustc")]
use crate::usefulness::{compute_match_usefulness, ValidityConstraint};

pub trait MatchCx: Sized + Clone + fmt::Debug {
    /// The type of a pattern.
    type Ty: Copy + Clone + fmt::Debug; // FIXME: remove Copy
    /// The index of an enum variant.
    type VariantIdx: Clone + Idx;
    /// A string literal
    type StrLit: Clone + PartialEq + fmt::Debug;
    /// Extra data to store on a match arm.
    type ArmData: Copy + Clone + fmt::Debug;
    /// Extra data to store on a pattern. `Default` needed when we create fictitious wildcard
    /// patterns during analysis.
    type PatData: Clone + Default;

    fn is_opaque_ty(ty: Self::Ty) -> bool;
    fn is_exhaustive_patterns_feature_on(&self) -> bool;

    /// The number of fields for this constructor.
    fn ctor_arity(&self, ctor: &Constructor<Self>, ty: Self::Ty) -> usize;

    /// The types of the fields for this constructor. The result must have a length of
    /// `ctor_arity()`.
    fn ctor_sub_tys(&self, ctor: &Constructor<Self>, ty: Self::Ty) -> &[Self::Ty];

    /// The set of all the constructors for `ty`.
    ///
    /// This must follow the invariants of `ConstructorSet`
    fn ctors_for_ty(&self, ty: Self::Ty) -> ConstructorSet<Self>;

    /// Best-effort `Debug` implementation.
    fn debug_pat(f: &mut fmt::Formatter<'_>, pat: &DeconstructedPat<'_, Self>) -> fmt::Result;

    /// Raise a bug.
    fn bug(&self, fmt: fmt::Arguments<'_>) -> !;
}

/// The arm of a match expression.
#[derive(Clone, Debug)]
pub struct MatchArm<'p, Cx: MatchCx> {
    /// The pattern must have been lowered through `check_match::MatchVisitor::lower_pattern`.
    pub pat: &'p DeconstructedPat<'p, Cx>,
    pub has_guard: bool,
    pub arm_data: Cx::ArmData,
}

impl<'p, Cx: MatchCx> Copy for MatchArm<'p, Cx> {}

pub trait Captures<'a> {}
impl<'a, T: ?Sized> Captures<'a> for T {}

/// The entrypoint for this crate. Computes whether a match is exhaustive and which of its arms are
/// useful, and runs some lints.
#[cfg(feature = "rustc")]
pub fn analyze_match<'p, 'tcx>(
    cx: &RustcMatchCheckCtxt<'p, 'tcx>,
    arms: &[rustc::MatchArm<'p, 'tcx>],
    scrut_ty: Ty<'tcx>,
) -> rustc::UsefulnessReport<'p, 'tcx> {
    // Arena to store the extra wildcards we construct during analysis.
    let wildcard_arena = cx.pattern_arena;
    let pat_column = PatternColumn::new(arms);

    let scrut_validity = ValidityConstraint::from_bool(cx.known_valid_scrutinee);
    let report = compute_match_usefulness(cx, arms, scrut_ty, scrut_validity, wildcard_arena);

    // Lint on ranges that overlap on their endpoints, which is likely a mistake.
    lint_overlapping_range_endpoints(cx, &pat_column, wildcard_arena);

    // Run the non_exhaustive_omitted_patterns lint. Only run on refutable patterns to avoid hitting
    // `if let`s. Only run if the match is exhaustive otherwise the error is redundant.
    if cx.refutable && report.non_exhaustiveness_witnesses.is_empty() {
        lint_nonexhaustive_missing_variants(cx, arms, &pat_column, scrut_ty, wildcard_arena)
    }

    report
}
