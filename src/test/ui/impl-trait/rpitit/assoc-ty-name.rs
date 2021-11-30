// check-pass
trait Foo {
    fn bar() -> impl Send;
}

fn foo<T: Foo>(t: T)
where
    <T as Foo>::bar: Send,
{
}

fn main() {}
