//@ check-pass

#![feature(ergonomic_clones)]

fn ergonomic_clone_closure() -> i32 {
    let cl = use || {
        1
    };
    cl()
}

fn main() {}
