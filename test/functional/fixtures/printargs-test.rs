//! Functional test helper that prints command-line arguments (`test/functional/fixtures/printargs-test.c`).

use std::env;

fn format_args<I, S>(args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = String::new();
    for (idx, arg) in args.into_iter().enumerate() {
        out.push_str(&format!("arg{}={};", idx + 1, arg.as_ref()));
    }
    out
}

fn main() {
    print!("{}", format_args(env::args().skip(1)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_arguments_with_one_based_indices() {
        assert_eq!(format_args(["foo", "bar baz"]), "arg1=foo;arg2=bar baz;");
        assert_eq!(format_args::<[&str; 0], &str>([]), "");
    }
}
