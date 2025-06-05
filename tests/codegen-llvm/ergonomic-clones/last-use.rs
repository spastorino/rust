//@ compile-flags: -C no-prepopulate-passes -Copt-level=0 -Zmir-opt-level=0

#![crate_type = "lib"]

#![feature(ergonomic_clones)]
#![allow(incomplete_features)]

use std::clone::UseCloned;

#[derive(Clone)]
struct Foo;

impl UseCloned for Foo {}

pub fn ergonomic_clone_closure_use_cloned() -> Foo {
    let f = Foo;

    // CHECK: ; call <last_use::Foo as core::clone::Clone>::clone
    let f1 = use || f;

    // CHECK-NOT: ; call <last_use::Foo as core::clone::Clone>::clone
    let f2 = use || f;

    f2()
}
