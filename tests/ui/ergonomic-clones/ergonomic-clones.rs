//@ check-pass

#![feature(ergonomic_clones)]

 fn ergonomic_clone(x: i32) -> i32 {
     x.use.use.abs()
 }

 fn main() {}
