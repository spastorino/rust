#![feature(pattern_types, rustc_attrs)]
#![feature(core_pattern_type)]
#![feature(core_pattern_types)]
#![allow(incomplete_features)]

use std::pat::pattern_type;

#[rustc_layout(debug)]
type X = std::num::NonZeroU32; //~ ERROR layout_of
#[rustc_layout(debug)]
type Y = pattern_type!(u32 is 1..); //~ ERROR layout_of
#[rustc_layout(debug)]
type Z = Option<pattern_type!(u32 is 1..)>; //~ ERROR layout_of
#[rustc_layout(debug)]
type A = Option<std::num::NonZeroU32>; //~ ERROR layout_of
#[rustc_layout(debug)]
struct NonZeroU32New(pattern_type!(u32 is 1..)); //~ ERROR layout_of

fn main() {
    let x: pattern_type!(u32 is 1..) = unsafe { std::mem::transmute(42_u32) };
}
