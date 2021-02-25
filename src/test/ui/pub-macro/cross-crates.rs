// check-pass
// aux-build:cross-crates.rs
// compile-flags: -Zinstrument-coverage -Zdump-mir=InstrumentCoverage -Zdump-mir-spanview -Zdump-mir-dir=/tmp/

extern crate cross_crates;

fn main() {
    cross_crates::m::foo!();
}
