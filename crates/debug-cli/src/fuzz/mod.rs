//! `jade-debug fuzz`: random but repeatable input to a headless app, until it
//! crashes or hangs (#140). The same seed does the same things.

mod actions;
mod app;
mod rng;
mod run;

use std::path::PathBuf;

use crate::launch;
use app::Gone;
use run::Fault;

const DEFAULT_RUNS: u32 = 5;
const DEFAULT_STEPS: u32 = 200;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FuzzArgs {
    /// The first run's seed; each later run takes the next number.
    seed: u64,
    runs: u32,
    steps: u32,
    bin: Option<PathBuf>,
    /// Print every step as it is taken.
    trace: bool,
}

pub(crate) fn parse_args(words: &[String]) -> Result<FuzzArgs, String> {
    let mut args = FuzzArgs {
        seed: 1,
        runs: DEFAULT_RUNS,
        steps: DEFAULT_STEPS,
        bin: None,
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
            "--trace" => args.trace = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(args)
}

/// What is printed for one fault.
fn describe(fault: &Fault) -> String {
    let (what, text) = match &fault.gone {
        Gone::Crash(text) => ("crash", text),
        Gone::Hang(text) => ("hang", text),
    };
    format!(
        "[{what}] {}\n  after step {} of seed {}\n  repeat: jade-debug fuzz --seed {} --runs 1 --steps {}\n    {}",
        text.replace('\n', "\n    "),
        fault.step,
        fault.seed,
        fault.seed,
        fault.step.max(1),
        fault.trail.join("\n    "),
    )
}

/// Runs the fuzzer and prints each crash or hang; `Err` if there was one.
pub(crate) fn fuzz(args: FuzzArgs) -> Result<(), String> {
    let bin = launch::find_app(args.bin)?;
    println!("app: {}", bin.display());
    let mut faults = 0;
    for index in 0..args.runs {
        let seed = args.seed.wrapping_add(u64::from(index));
        // Printed at once: a later failure of the fuzzer must not lose it.
        if let Some(fault) = run::run(&bin, (seed, args.steps), args.trace)? {
            println!("{}\n", describe(&fault));
            faults += 1;
        }
        eprintln!("seed {seed} done ({} of {})", index + 1, args.runs);
    }
    println!("{faults} crashes or hangs in {} runs", args.runs);
    if faults > 0 {
        return Err("the app crashed or hung".into());
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
        let args = parse("--seed 9 --steps 30 --trace").unwrap();
        assert_eq!(
            (args.seed, args.runs, args.steps, args.trace),
            (9, DEFAULT_RUNS, 30, true)
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

    #[test]
    fn a_fault_names_the_command_that_repeats_it() {
        let text = describe(&Fault {
            gone: Gone::Crash("panicked at a.rs:1:1:\nbad".into()),
            seed: 17,
            step: 0,
            trail: vec!["1: key up".into()],
        });
        assert!(
            text.starts_with("[crash] panicked at a.rs:1:1:\n    bad"),
            "{text}"
        );
        assert!(
            text.contains("--seed 17 --runs 1 --steps 1\n    1: key up"),
            "{text}"
        );
    }
}
