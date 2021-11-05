// check-pass
// edition:2018

trait MyTrait {
    async fn foo(&self) -> i32;
}

// TODO: This is a temporary test to make steps simpler.
// We should get rid of this test before this PR lands.
fn foo<T: MyTrait>()
where T::__Assoc: Send,
{
}

fn main() {}
