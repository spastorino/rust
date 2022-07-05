// check-pass

#![feature(return_position_impl_trait_v2)]

fn foo(x: &impl ?Sized) {}

fn main() {}
