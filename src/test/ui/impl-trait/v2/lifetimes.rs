// check-pass

#![allow(warnings)]
#![feature(return_position_impl_trait_v2)]

use std::fmt::Debug;

// fn nested_lifetime<'a>(
//     input: &'a str,
// ) -> impl Iterator<Item = impl Iterator<Item = i32> + 'a> + 'a {
//     input.lines().map(|line| line.split_whitespace().map(|cell| cell.parse().unwrap()))
// }

trait MultiRegionTrait<'a, 'b>: Debug {}

#[derive(Debug)]
struct MultiRegionStruct<'a, 'b>(&'a u32, &'b u32);
impl<'a, 'b> MultiRegionTrait<'a, 'b> for MultiRegionStruct<'a, 'b> {}

fn finds_explicit_bound<'a: 'b, 'b>(x: &'a u32, y: &'b u32) -> impl MultiRegionTrait<'a, 'b> + 'b {
    MultiRegionStruct(x, y)
}
fn main() {}
