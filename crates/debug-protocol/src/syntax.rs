//! How checks and rects are written as text, shared by `jade-debug expect` and
//! the acceptance catalog.

use std::str::FromStr;

use crate::{Check, DEFAULT_TOLERANCE, Rect};

/// `text` as exactly `count` comma-separated numbers; `what` names them in the
/// error.
pub fn numbers(text: &str, count: usize, what: &str) -> Result<Vec<f32>, String> {
    let values: Result<Vec<f32>, _> = text.split(',').map(str::parse).collect();
    match values {
        Ok(v) if v.len() == count => Ok(v),
        _ => Err(format!(
            "{what} must be {count} comma-separated numbers, got {text:?}"
        )),
    }
}

/// `X,Y,W,H`.
impl FromStr for Rect {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let v = numbers(text, 4, "a rect (X,Y,W,H)")?;
        Ok(Rect {
            x: v[0],
            y: v[1],
            w: v[2],
            h: v[3],
        })
    }
}

/// `ink:X,Y,W,H[,MIN%]`, `empty:X,Y,W,H[,MAX%]`, `color:X,Y,#rrggbb[,TOL]` or
/// `within:X,Y,W,H`.
impl FromStr for Check {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, String> {
        parse_check(word)
    }
}

fn parse_check(word: &str) -> Result<Check, String> {
    let (name, args) = word
        .split_once(':')
        .ok_or_else(|| format!("{word:?} is not a check: use ink:, empty:, color: or within:"))?;
    let parts: Vec<&str> = args.split(',').collect();
    let percent = |extra: Option<&&str>, default: f32| {
        extra.map_or(Ok(default), |p| {
            p.trim_end_matches('%')
                .parse()
                .map_err(|_| format!("{p:?} is not a percentage"))
        })
    };
    let area = |parts: &[&str]| parts[..parts.len().min(4)].join(",").parse::<Rect>();
    match name {
        "ink" if parts.len() == 4 || parts.len() == 5 => Ok(Check::InkIn {
            rect: area(&parts)?,
            min_percent: percent(parts.get(4), 0.1)?,
        }),
        "empty" if parts.len() == 4 || parts.len() == 5 => Ok(Check::Empty {
            rect: area(&parts)?,
            max_percent: percent(parts.get(4), 0.0)?,
        }),
        "within" if parts.len() == 4 => Ok(Check::BboxWithin {
            rect: area(&parts)?,
        }),
        "color" if parts.len() == 3 || parts.len() == 4 => {
            let v = numbers(&parts[..2].join(","), 2, "color:X,Y,#rrggbb")?;
            Ok(Check::Color {
                x: v[0],
                y: v[1],
                color: parts[2].into(),
                tolerance: parts.get(3).map_or(Ok(DEFAULT_TOLERANCE), |t| {
                    t.parse()
                        .map_err(|_| format!("{t:?} is not a tolerance (0-255)"))
                })?,
            })
        }
        _ => Err(format!(
            "bad check {word:?}: ink:X,Y,W,H[,MIN%] empty:X,Y,W,H[,MAX%] \
             color:X,Y,#rrggbb[,TOL] within:X,Y,W,H"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_parse_from_their_written_form() {
        let rect = Rect {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
        };
        assert_eq!(
            "ink:1,2,3,4,5%".parse(),
            Ok(Check::InkIn {
                rect,
                min_percent: 5.0
            })
        );
        assert_eq!("within:1,2,3,4".parse(), Ok(Check::BboxWithin { rect }));
        for bad in ["ink", "ink:1,2,3", "glow:1,2,3,4", "color:1,2"] {
            assert!(bad.parse::<Check>().is_err(), "{bad}");
        }
    }
}
