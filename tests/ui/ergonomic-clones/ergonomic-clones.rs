//@ edition:2024
//@ compile-flags: -Zunstable-options
//@ check-pass

 fn ergonomic_clone(x: i32) -> i32 {
     x.use.use.abs()
 }

 fn main() {}
