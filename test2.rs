// check-pass

#![feature(return_position_impl_trait_in_trait)]
#![allow(incomplete_features)]

use std::fmt::Debug;

trait Foo {
    fn foo<T>(&self) -> impl Debug;
}

impl Foo for String {
    fn foo<T>(&self) -> impl Debug {
        (format!("{} {}", self, std::any::type_name::<T>()),)
    }
}

fn main() {
    let s = "hello world!".to_string();
    println!("{:?}", s.foo::<u64>());
}
