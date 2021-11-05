// check-pass
// edition:2018

trait MyTrait {
    async fn foo(&self) -> i32;
}

async fn foo<T: MyTrait>(t: T) -> i32 {
    t.foo().await
}

fn main() {}
