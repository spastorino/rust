// check-pass

#![feature(return_position_impl_trait_v2)]

trait MyTrait {}

impl<T> MyTrait for T {}

fn parser<T>(t: T) -> impl MyTrait {
    t
}

fn consume_parser<T: MyTrait>(_t: T) {}

fn main() {
    consume_parser(parser(22));
}
