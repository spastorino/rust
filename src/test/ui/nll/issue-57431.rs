#![feature(nll)]

struct X;
impl X {
    fn mutate(&mut self) {}
}

fn main() {
    let mut term = X;
    let ref_term = if true {
        &mut term
    } else {
        &X
    };
    ref_term.mutate();
}
