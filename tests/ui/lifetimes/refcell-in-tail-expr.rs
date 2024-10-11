//@ revisions: edition2021 edition2024
//@ [edition2021] edition: 2021
//@ [edition2024] edition: 2024
//@ [edition2024] compile-flags: -Zunstable-options
//@ [edition2024] check-pass

#![cfg_attr(edition2024, feature(shorter_tail_lifetimes))]

fn main() {
    let cell = std::cell::RefCell::new(0u8);

    if let Ok(mut byte) = cell.try_borrow_mut() {
        //[edition2021]~^ ERROR: `cell` does not live long enough
        *byte = 1;
    }
}
