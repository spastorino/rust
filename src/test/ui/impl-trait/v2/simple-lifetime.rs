// check-pass

#![feature(return_position_impl_trait_v2)]

trait MyTrait<T> {}

struct MyStruct<T> {
    t: T,
}

impl<T> MyTrait<T> for MyStruct<&u32> {}

fn foo<'a, A>() -> impl MyTrait<&'a A> {
    MyStruct { t: &0 }
}

fn main() {}
