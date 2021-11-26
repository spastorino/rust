// check-pass

trait Stream {}

trait Parser {
    type Input;
    type Output: Parseable;
}

trait Parseable {
    fn parser<I: Stream, O: Parseable>() -> impl Parser<Input = I, Output = O>;
}

fn main() {}
