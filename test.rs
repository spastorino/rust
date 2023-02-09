#![feature(return_position_impl_trait_in_trait)]
#![allow(incomplete_features)]

trait Bar {}

trait Foo {
    fn test() -> impl Bar;
    //                ^^^ required by this bound in `Foo::test::{opaque#0}`
}

impl Foo for () {
    fn test() {}
    //        ^ the trait `Bar` is not implemented for `()`
}

fn main() {}
