//! Reading `[font]`, `[shape]` and `[heading]` from `config.toml` (#58). Every
//! key is optional and judged on its own: a missing, mistyped or out-of-range
//! value gives that key its default (clamped when it is a number) and one
//! notice for stderr; the rest of the file is still used. Never panics on user
//! data. Tables other steps own (and keys outside these tables) are ignored.

use toml::{Table, Value};

use crate::theme::ConfigDirs;

use super::typography::{FontFamily, HEADING_LEVELS, Radius, Typography};

/// What reading produced: the settings and what to tell the user.
#[derive(Debug, PartialEq)]
pub struct Read {
    pub typography: Typography,
    pub notices: Vec<String>,
}

impl Typography {
    /// The settings `src` (a `config.toml`) asks for, on top of the defaults.
    pub fn from_config(src: &str) -> Read {
        let mut reader = Reader {
            notices: Vec::new(),
        };
        let mut typography = Typography::default();
        let table = match src.parse::<Table>() {
            Ok(table) => table,
            Err(e) => {
                reader.notices.push(format!("config.toml: {e}"));
                return Read {
                    typography,
                    notices: reader.notices,
                };
            }
        };
        reader.font(&table, &mut typography);
        reader.shape(&table, &mut typography);
        reader.heading(&table, &mut typography);
        reader.blocks(&table, &mut typography);
        Read {
            typography,
            notices: reader.notices,
        }
    }
}

/// The settings in `config.toml` under `dirs`; the defaults when there is no
/// file or it cannot be used (with a notice for the latter).
pub fn load(dirs: Option<&ConfigDirs>) -> Read {
    match crate::theme::config_text(dirs) {
        None => Read {
            typography: Typography::default(),
            notices: Vec::new(),
        },
        Some(Ok(src)) => Typography::from_config(&src),
        Some(Err(message)) => Read {
            typography: Typography::default(),
            notices: vec![format!("config.toml: {message}")],
        },
    }
}

mod blocks;

struct Reader {
    notices: Vec<String>,
}

/// The accepted range of a number key, inclusive.
type Range = (f32, f32);

const SIZE: Range = (8.0, 40.0);
const LINE_HEIGHT: Range = (1.0, 3.0);
const MATH_SCALE: Range = (0.5, 3.0);
const RADIUS: Range = (0.0, 64.0);
const BORDER: Range = (0.0, 8.0);
const SCALE: Range = (0.5, 4.0);
const WEIGHT: Range = (100.0, 900.0);
const SPACE: Range = (0.0, 4.0);

impl Reader {
    fn notice(&mut self, message: String) {
        self.notices.push(format!("config.toml: {message}"));
    }

    /// The table `name`, or `None` (with a notice when it is not a table).
    fn section<'a>(&mut self, root: &'a Table, name: &str, known: &[&str]) -> Option<&'a Table> {
        let value = root.get(name)?;
        let Some(table) = value.as_table() else {
            self.notice(format!("[{name}] is not a table, using the defaults"));
            return None;
        };
        for key in table.keys().filter(|k| !known.contains(&k.as_str())) {
            self.notice(format!("unknown key {name}.{key}"));
        }
        Some(table)
    }

    /// A number key clamped into `range`; `default` when absent or not a number.
    fn number(
        &mut self,
        (table, name): (&Table, &str),
        key: &str,
        range: Range,
        default: f32,
    ) -> f32 {
        match table.get(key) {
            None => default,
            Some(v) => match as_f32(v) {
                Some(n) => self.clamped(&format!("{name}.{key}"), n, range),
                None => {
                    self.notice(format!("{name}.{key} must be a number, using {default}"));
                    default
                }
            },
        }
    }

    fn clamped(&mut self, path: &str, n: f32, (lo, hi): Range) -> f32 {
        if n.is_nan() {
            self.notice(format!("{path} is not a number"));
            return lo;
        }
        let kept = n.clamp(lo, hi);
        if kept != n {
            self.notice(format!("{path} = {n} is outside {lo}..{hi}, using {kept}"));
        }
        kept
    }

    fn family(
        &mut self,
        (table, name): (&Table, &str),
        key: &str,
        default: FontFamily,
    ) -> FontFamily {
        match table.get(key).map(Value::as_str) {
            None => default,
            Some(Some(f)) if f.eq_ignore_ascii_case("IBM Plex Mono") => FontFamily::IbmPlexMono,
            Some(_) => {
                self.notice(format!(
                    "{name}.{key}: only \"IBM Plex Mono\" is supported for now, using it"
                ));
                default
            }
        }
    }

    fn font(&mut self, root: &Table, t: &mut Typography) {
        let known = ["family", "code_family", "size", "line_height", "math_scale"];
        let Some(table) = self.section(root, "font", &known) else {
            return;
        };
        let at = (table, "font");
        t.font.family = self.family(at, "family", t.font.family);
        t.font.code_family = self.family(at, "code_family", t.font.code_family);
        t.font.size = self.number(at, "size", SIZE, t.font.size);
        t.font.line_height = self.number(at, "line_height", LINE_HEIGHT, t.font.line_height);
        t.font.math_scale = self.number(at, "math_scale", MATH_SCALE, t.font.math_scale);
    }

    fn shape(&mut self, root: &Table, t: &mut Typography) {
        let Some(table) = self.section(root, "shape", &["radius", "border_width"]) else {
            return;
        };
        if let Some(value) = table.get("radius") {
            t.shape.radius = self.radius("shape.radius", value, t.shape.radius);
        }
        t.shape.border_width = self.number(
            (table, "shape"),
            "border_width",
            BORDER,
            t.shape.border_width,
        );
    }

    /// `"full"` or px; `None` (with a notice) for anything else.
    fn try_radius(&mut self, path: &str, value: &Value) -> Option<Radius> {
        if value
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("full"))
        {
            return Some(Radius::Full);
        }
        match as_f32(value) {
            Some(n) => Some(Radius::Px(self.clamped(path, n, RADIUS))),
            None => {
                self.notice(format!(
                    "{path} must be a number or \"full\", using the default"
                ));
                None
            }
        }
    }

    fn radius(&mut self, path: &str, value: &Value, default: Radius) -> Radius {
        self.try_radius(path, value).unwrap_or(default)
    }

    fn heading(&mut self, root: &Table, t: &mut Typography) {
        let known = ["scale", "weight", "space_above", "space_below", "rule"];
        let Some(table) = self.section(root, "heading", &known) else {
            return;
        };
        let h = &mut t.heading;
        self.levels(table, "scale", &mut h.scale, SCALE);
        self.levels(table, "space_above", &mut h.space_above, SPACE);
        self.levels(table, "space_below", &mut h.space_below, SPACE);
        let mut weight = h.weight.map(f32::from);
        self.levels(table, "weight", &mut weight, WEIGHT);
        h.weight = weight.map(|w| w.round() as u16);
        let Some(value) = table.get("rule") else {
            return;
        };
        let Some(items) = value.as_array() else {
            self.notice("heading.rule must be an array of true/false, using the default".into());
            return;
        };
        let default = h.rule;
        h.rule = self.spread("heading.rule", items, default, |_, v| v.as_bool());
    }

    /// A per-level number array: 1 to 6 entries, the last repeated; a bad
    /// entry keeps its level's default.
    fn levels(&mut self, table: &Table, key: &str, out: &mut [f32; HEADING_LEVELS], range: Range) {
        let Some(value) = table.get(key) else {
            return;
        };
        let path = format!("heading.{key}");
        let Some(items) = value.as_array() else {
            self.notice(format!(
                "{path} must be an array of numbers, using the default"
            ));
            return;
        };
        let default = *out;
        *out = self.spread(&path, items, default, |reader, v| {
            as_f32(v).map(|n| reader.clamped(&path, n, range))
        });
    }

    /// Fills `N` slots from `items`; see [`Reader::levels`]. `parse` returns
    /// `None` for a bad entry.
    fn spread<T: Copy, const N: usize>(
        &mut self,
        path: &str,
        items: &[Value],
        default: [T; N],
        mut parse: impl FnMut(&mut Reader, &Value) -> Option<T>,
    ) -> [T; N] {
        if items.is_empty() {
            self.notice(format!("{path} is empty, using the default"));
            return default;
        }
        if items.len() > N {
            self.notice(format!(
                "{path} has {} entries, only {N} are used",
                items.len()
            ));
        }
        let mut out = default;
        // Each entry is judged once; the last one then covers the levels after it.
        let mut last = None;
        for (level, item) in items.iter().take(N).enumerate() {
            last = parse(self, item);
            match last {
                Some(v) => out[level] = v,
                None => self.notice(format!(
                    "{path}[{level}] has the wrong type, using the default"
                )),
            }
        }
        if let Some(v) = last {
            out[items.len().min(N)..].fill(v);
        }
        out
    }
}

/// TOML integers and floats alike.
fn as_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Float(f) => Some(*f as f32),
        Value::Integer(i) => Some(*i as f32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(src: &str) -> Read {
        Typography::from_config(src)
    }

    #[test]
    fn an_empty_or_unrelated_file_gives_the_defaults_and_no_notice() {
        for src in [
            "",
            "vault = \"/v\"\ntheme = \"x\"\njustify = true\n",
            "[something_else]\nradius = 4\n",
        ] {
            let r = read(src);
            assert_eq!(r.typography, Typography::default(), "{src:?}");
            assert!(r.notices.is_empty(), "{src:?}: {:?}", r.notices);
        }
    }

    #[test]
    fn valid_values_are_read() {
        let r = read(
            "[font]\nsize = 18\nline_height = 1.7\nmath_scale = 1.1\nfamily = \"ibm plex mono\"\n\
             [shape]\nradius = 8\nborder_width = 2.5\n\
             [heading]\nscale = [2.0, 1.6, 1.35, 1.15, 1.0, 0.9]\nweight = [800, 700]\n\
             space_above = [1, 0.5]\nspace_below = [0.25]\nrule = [true, true, false]\n",
        );
        assert_eq!(r.notices, Vec::<String>::new());
        let t = r.typography;
        assert_eq!(
            (t.font.size, t.font.line_height, t.font.math_scale),
            (18.0, 1.7, 1.1)
        );
        assert_eq!(
            (t.shape.radius, t.shape.border_width),
            (Radius::Px(8.0), 2.5)
        );
        assert_eq!(t.heading.scale, [2.0, 1.6, 1.35, 1.15, 1.0, 0.9]);
        // Short arrays repeat their last value.
        assert_eq!(t.heading.weight, [800, 700, 700, 700, 700, 700]);
        assert_eq!(t.heading.space_above, [1.0, 0.5, 0.5, 0.5, 0.5, 0.5]);
        assert_eq!(t.heading.space_below, [0.25; HEADING_LEVELS]);
        assert_eq!(t.heading.rule, [true, true, false, false, false, false]);
    }

    #[test]
    fn a_radius_may_be_full() {
        assert_eq!(
            read("[shape]\nradius = \"Full\"\n").typography.shape.radius,
            Radius::Full
        );
        let r = read("[shape]\nradius = \"round\"\n");
        assert_eq!(
            r.typography.shape.radius,
            Typography::default().shape.radius
        );
        assert_eq!(r.notices.len(), 1);
    }

    #[test]
    fn out_of_range_numbers_are_clamped_with_a_notice() {
        let r = read(
            "[font]\nsize = 200\nline_height = 0.1\n[heading]\nscale = [9, 0.01]\n[shape]\nradius = -4\n",
        );
        let t = r.typography;
        assert_eq!((t.font.size, t.font.line_height), (40.0, 1.0));
        assert_eq!(t.heading.scale[..2], [4.0, 0.5]);
        assert_eq!(t.shape.radius, Radius::Px(0.0));
        assert_eq!(r.notices.len(), 5, "{:?}", r.notices);
    }

    #[test]
    fn a_wrong_type_falls_back_for_that_key_only() {
        let r = read(
            "[font]\nsize = \"big\"\nline_height = 1.8\n[heading]\nscale = 2\nweight = [700, \"x\", 600]\n",
        );
        let t = r.typography;
        let d = Typography::default();
        assert_eq!(t.font.size, d.font.size);
        assert_eq!(t.font.line_height, 1.8);
        assert_eq!(t.heading.scale, d.heading.scale);
        // Level 2's entry is bad: its default stays, the others are read.
        assert_eq!(t.heading.weight, [700, 700, 600, 600, 600, 600]);
        assert_eq!(r.notices.len(), 3, "{:?}", r.notices);
    }

    #[test]
    fn another_font_family_is_refused() {
        let r = read("[font]\nfamily = \"Inter\"\ncode_family = 3\n");
        assert_eq!(r.typography.font, Typography::default().font);
        assert_eq!(r.notices.len(), 2);
    }

    #[test]
    fn unknown_keys_and_a_non_table_are_noticed() {
        let r = read("[font]\nsise = 12\n[heading]\nrule = true\n[shape]\n");
        assert_eq!(r.typography, Typography::default());
        assert_eq!(r.notices.len(), 2, "{:?}", r.notices);
        let r = read("font = 3\n");
        assert_eq!(r.typography, Typography::default());
        assert_eq!(r.notices.len(), 1);
    }

    #[test]
    fn empty_and_overlong_arrays_are_handled() {
        let r = read("[heading]\nscale = []\nweight = [100, 200, 300, 400, 500, 600, 700, 800]\n");
        let t = r.typography;
        assert_eq!(t.heading.scale, Typography::default().heading.scale);
        assert_eq!(t.heading.weight, [100, 200, 300, 400, 500, 600]);
        assert_eq!(r.notices.len(), 2);
    }

    #[test]
    fn a_file_that_does_not_parse_gives_the_defaults_and_one_notice() {
        let r = read("[font\nsize = ");
        assert_eq!(r.typography, Typography::default());
        assert_eq!(r.notices.len(), 1);
    }

    #[test]
    fn load_reads_the_file_in_the_config_dir() {
        let dir = std::env::temp_dir().join(format!("jade-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dirs = ConfigDirs::new(dir.clone());
        assert_eq!(
            load(Some(&dirs)).typography,
            Typography::default(),
            "no file"
        );
        assert_eq!(load(None).typography, Typography::default(), "no dir");
        std::fs::write(dirs.config_file(), "[font]\nsize = 20\n").unwrap();
        assert_eq!(load(Some(&dirs)).typography.font.size, 20.0);
        std::fs::write(dirs.config_file(), [0xff, 0xff]).unwrap();
        let bad = load(Some(&dirs));
        assert_eq!(bad.typography, Typography::default());
        assert_eq!(bad.notices.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hostile_numbers_never_panic() {
        let src = "[font]\nsize = nan\nline_height = inf\n[heading]\nscale = [-inf, nan]\nweight = [1e30]\n\
                   [shape]\nradius = 9223372036854775807\n";
        let r = read(src);
        let t = r.typography;
        assert!(t.font.size.is_finite() && t.font.line_height.is_finite());
        assert!(t.heading.scale.iter().all(|s| (0.5..=4.0).contains(s)));
        assert_eq!(t.heading.weight, [900; HEADING_LEVELS]);
    }
}
