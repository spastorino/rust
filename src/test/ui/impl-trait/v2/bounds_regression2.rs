// check-pass

#![feature(return_position_impl_trait_v2)]

pub trait FakeGenerator {
    type Yield;
    type Return;
}

pub trait FakeFuture {
    type Output;
}

pub fn future_from_generator<T>(x: T) -> impl FakeFuture<Output = T::Return>
where
    T: FakeGenerator<Yield = ()>,
{
    GenFuture(x)
}

struct GenFuture<T>(T)
where
    T: FakeGenerator<Yield = ()>;

impl<T> FakeFuture for GenFuture<T>
where
    T: FakeGenerator<Yield = ()>,
{
    type Output = T::Return;
}

fn main() {}
