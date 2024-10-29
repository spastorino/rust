fn ergonomic_clone(x: i32) -> i32 {
    x.use
    //~^ ERROR expected identifier, found keyword `use`
    //~| ERROR `i32` is a primitive type and therefore doesn't have fields [E0610]
}

fn main() {}
