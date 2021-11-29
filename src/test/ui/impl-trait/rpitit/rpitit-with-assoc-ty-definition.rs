// check-pass

trait NewIntoIterator {
    type Item;
    fn into_iter(self) -> impl Iterator<Item = Self::Item>;
}

fn bar<T: NewIntoIterator<Item = u32>>(t: T) -> Option<u32> {
    t.into_iter().next()
}

fn main() {}
