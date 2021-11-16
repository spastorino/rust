// check-pass

trait MyTrait {
    fn foo() -> impl Send;
}

// TODO: This is a temporary test to make steps simpler.
// We should get rid of this test before this PR lands.
fn foo<T: MyTrait>()
where
    T::__Assoc: Send,
{
}

// TODO: Make this case below work
//fn bar<T: MyTrait>() {
//    foo::<T>();
//}

fn main() {}
