fn ergonomic_clone(x: i32) -> i32 {
    x.use
    //~^ ERROR ergonomic clones is experimental [E0658]
}

fn main() {}
