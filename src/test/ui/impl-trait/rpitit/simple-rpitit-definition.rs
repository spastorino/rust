// check-pass

trait MyTrait {
    fn foo(&self) -> impl Send;
}

fn bar(f: impl MyTrait) -> impl Send {
    f.foo()
}

fn main() {}
