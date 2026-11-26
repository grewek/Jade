//! Spike for #133: compares ways of building Jade's block tree.
//!
//! `parser-spike <cold|edits|ranges|fuzz|cases|spec> [files...]`

mod backends;
mod bench;
mod check;
mod own;
mod tree;

use backends::CANDIDATES;

/// xorshift64*: seeded, so every run edits the same places.
pub struct Rng(pub u64);

impl Rng {
    pub fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 33) as usize % bound.max(1)
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, files) = match args.split_first() {
        Some((command, files)) => (command.as_str(), files),
        None => ("", &[][..]),
    };
    if command == "cases" {
        for (name, text) in check::CASES {
            println!("## {name}");
            for candidate in CANDIDATES {
                let units = candidate.1().parse(text);
                println!("  {:13} {}", candidate.0, check::outline(text, &units));
            }
        }
        return Ok(());
    }
    for file in files {
        if command == "spec" {
            for candidate in CANDIDATES {
                print!(
                    "{}: {}",
                    candidate.0,
                    check::spec(candidate, file.as_ref())?
                );
            }
            continue;
        }
        let text = std::fs::read_to_string(file)?;
        println!("== {command} {file} ({} B)", text.len());
        for candidate in CANDIDATES {
            match command {
                "cold" => bench::cold(candidate, &text),
                "edits" => {
                    for (label, typing) in bench::TYPINGS {
                        bench::edits(candidate, &text, label, *typing);
                    }
                }
                "ranges" => {
                    let units = candidate.1().parse(&text);
                    println!("{:13} {}", candidate.0, check::ranges(&text, &units));
                }
                "fuzz" => {
                    let (mismatches, first) = check::fuzz(candidate, &text, 400);
                    println!(
                        "{:13} {mismatches} mismatches in 40000 edits\n{first}",
                        candidate.0
                    );
                }
                _ => eprintln!("unknown command {command:?}"),
            }
        }
    }
    Ok(())
}
