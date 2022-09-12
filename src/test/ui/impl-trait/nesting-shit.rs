// check-pass

fn parse_templates<'a, I: Iterator<Item = &'a u32> + 'a>(
    paths: I,
) -> impl Iterator<Item = Result<impl Iterator<Item = (u32, u32)>, String>> + 'a {
    paths.map(|_| Ok(vec![(1, 2)].into_iter().map(|(_, _)| (11, 22))))
}

fn main() {}
