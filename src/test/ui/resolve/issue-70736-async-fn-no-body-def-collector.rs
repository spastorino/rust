// edition:2018

async fn free(); //~ ERROR without a body

struct A;
impl A {
    async fn inherent(); //~ ERROR without body
}

trait B {
    async fn associated();
}
impl B for A {
    //~^ ERROR not all trait items implemented, missing: `__Assoc` [E0046]
    async fn associated(); //~ ERROR without body
    //~^ ERROR incompatible type for trait
}

fn main() {}
