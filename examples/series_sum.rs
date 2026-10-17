//! A reproducible series-construction workload with large symbolic coefficients.
use std::{env, time::Instant};
use symbolica::prelude::*;

fn main() {
    let count = env::args().nth(1).map_or(10_000, |n| n.parse().unwrap());
    let x = symbol!("x");
    let f = symbol!("f");
    let coefficients = (0..count)
        .map(|i| {
            let mut term = FunctionBuilder::new(f).add_arg(i);
            for j in 0..32 {
                term = term.add_arg(j);
            }
            term.finish()
        })
        .collect::<Vec<_>>();
    let input = Atom::add_many(coefficients.iter().map(|c| c * Atom::var(x)));
    let started = Instant::now();
    let result = input.series(x, 0, 4).unwrap();
    println!("{count} terms: {:?}", started.elapsed());
    assert_eq!(
        result.coefficient(1.into()),
        Some(Atom::add_many(&coefficients))
    );
}
