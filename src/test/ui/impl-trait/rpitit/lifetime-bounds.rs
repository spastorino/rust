trait Foo {}

impl<'x> Foo for &'x mut i32 {}

trait Bar {
    fn bar<'a>(x: &'a mut i32) -> impl Foo + 'a;
}

fn muh<'b, T: Bar>(y: &'b mut i32) {
    let a = T::bar(y);
    let b = T::bar(y);
    //~^ ERROR: cannot borrow `*y` as mutable more than once at a time
    let c = a;
}

fn main() {}
