//! `jade-debug fuzz`: random but repeatable input to a headless app, checked
//! after every step (#140). The same seed does the same things.

mod actions;
mod app;
mod findings;
mod rng;
mod run;

use std::path::PathBuf;

use crate::launch;

const DEFAULT_RUNS: u32 = 5;
const DEFAULT_STEPS: u32 = 200;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FuzzArgs {
    /// The first run's seed; each later run takes the next number.
    seed: u64,
    runs: u32,
    steps: u32,
    bin: Option<PathBuf>,
    /// Only a crash or a hang fails; everything else is just reported.
    crashes_only: bool,
    /// Print every step as it is taken.
    trace: bool,
}

pub(crate) fn parse_args(words: &[String]) -> Result<FuzzArgs, String> {
    let mut args = FuzzArgs {
        seed: 1,
        runs: DEFAULT_RUNS,
        steps: DEFAULT_STEPS,
        bin: None,
        crashes_only: false,
        trace: false,
    };
    let mut words = words.iter();
    while let Some(flag) = words.next() {
        let mut value = || words.next().ok_or_else(|| format!("{flag} needs a value"));
        let number = |text: &String| {
            text.parse::<u64>()
                .map_err(|_| format!("{flag}: {text:?} is not a number"))
        };
        let count = |text: &String| {
            number(text).and_then(|n| {
                u32::try_from(n)
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("{flag}: {n} is out of range"))
            })
        };
        match flag.as_str() {
            "--seed" => args.seed = number(value()?)?,
            "--runs" => args.runs = count(value()?)?,
            "--steps" => args.steps = count(value()?)?,
            "--bin" => args.bin = Some(value()?.into()),
            "--crashes-only" => args.crashes_only = true,
            "--trace" => args.trace = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(args)
}

/// Runs the fuzzer and prints what it found; `Err` when that fails the run.
pub(crate) fn fuzz(args: FuzzArgs) -> Result<(), String> {
    let bin = launch::find_app(args.bin)?;
    println!("app: {}", bin.display());
    let mut findings = findings::Findings::default();
    let mut broken = Ok(());
    for index in 0..args.runs {
        let seed = args.seed.wrapping_add(u64::from(index));
        broken = run::run(&bin, (seed, args.steps), args.trace, &mut findings);
        if broken.is_err() {
            break;
        }
        eprintln!("seed {seed} done ({} of {})", index + 1, args.runs);
    }
    // Also after the fuzzer failed: what it found until then still counts.
    println!("{}", findings.report());
    broken?;
    if findings.fails(args.crashes_only) {
        return Err("the fuzzer found faults".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &str) -> Result<FuzzArgs, String> {
        let words: Vec<String> = words.split_whitespace().map(str::to_owned).collect();
        parse_args(&words)
    }

    #[test]
    fn flags_set_their_values_and_the_rest_stay_default() {
        let args = parse("--seed 9 --steps 30 --crashes-only").unwrap();
        assert_eq!(
            (
                args.seed,
                args.runs,
                args.steps,
                args.crashes_only,
                args.trace
            ),
            (9, DEFAULT_RUNS, 30, true, false)
        );
    }

    #[test]
    fn bad_values_are_refused() {
        for bad in [
            "--seed",
            "--runs 0",
            "--steps x",
            "--steps 99999999999",
            "--fast",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
