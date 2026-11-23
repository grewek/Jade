//! Matrices, `cases`, `array`, `aligned` and friends: cells set in a grid
//! centred on the math axis, with fences sized like `\left … \right`.

use super::super::ast::{Col, Env, Environment, Fence, Font, Node};
use super::{Layout, Level, Style, Typesetter};

/// A row is at least this tall and deep (TeX's 0.7 and 0.3 of a 1.2 em
/// baseline skip), so a row of small cells does not collapse.
const STRUT_ASCENT: f32 = 0.84;
const STRUT_DESCENT: f32 = 0.36;
/// Extra space between the rows of `aligned` and `gathered` (amsmath's jot).
const JOT: f32 = 0.3;
/// Space between ordinary columns, and between the `r l` pairs of `aligned`.
const COL_GAP: f32 = 1.0;
const PAIR_GAP: f32 = 2.0;
/// Air between a fence and the cells.
const FENCE_PAD: f32 = 0.1;
/// Room a vertical rule takes at the edge of a table.
const EDGE_RULE_PAD: f32 = 0.3;

/// How the columns of a table are aligned and spaced, and where vertical
/// rules go (the index of the column each one sits before).
struct Plan {
    aligns: Vec<Col>,
    gaps: Vec<f32>,
    rules: Vec<usize>,
}

fn plan(env: &Environment, columns: usize) -> Plan {
    let mut aligns: Vec<Col> = Vec::with_capacity(columns);
    let mut rules = Vec::new();
    if env.kind == Env::Array {
        for col in &env.cols {
            match col {
                Col::Rule => rules.push(aligns.len()),
                other => aligns.push(*other),
            }
        }
    }
    let gap_before = |i: usize| match env.kind {
        _ if i == 0 => 0.0,
        Env::Aligned if i % 2 == 1 => 0.0,
        Env::Aligned => PAIR_GAP,
        _ => COL_GAP,
    };
    let fill = |i: usize| match env.kind {
        Env::Aligned if i.is_multiple_of(2) => Col::Right,
        Env::Aligned | Env::Cases => Col::Left,
        _ => Col::Center,
    };
    aligns.truncate(columns);
    while aligns.len() < columns {
        aligns.push(fill(aligns.len()));
    }
    Plan {
        gaps: (0..columns).map(gap_before).collect(),
        aligns,
        rules,
    }
}

/// The style the cells are set in: text style in arrays, display in `aligned`,
/// the script style in `smallmatrix`.
fn cell_style(kind: Env, outer: Style) -> Style {
    match kind {
        Env::Aligned | Env::Gathered => Style::new(Level::Display),
        Env::SmallMatrix if !outer.is_script() => Style::new(Level::Script),
        Env::Substack | Env::SmallMatrix => outer,
        _ if outer.is_script() => outer,
        _ => Style::new(Level::Text),
    }
}

fn fence_chars(kind: Env) -> (Option<char>, Option<char>) {
    match kind {
        Env::Matrix(Fence::Paren) => (Some('('), Some(')')),
        Env::Matrix(Fence::Bracket) => (Some('['), Some(']')),
        Env::Matrix(Fence::Brace) => (Some('{'), Some('}')),
        Env::Matrix(Fence::Bar) => (Some('|'), Some('|')),
        Env::Matrix(Fence::DoubleBar) => (Some('\u{2016}'), Some('\u{2016}')),
        Env::Cases => (Some('{'), None),
        _ => (None, None),
    }
}

impl Typesetter<'_> {
    pub(super) fn environment(
        &self,
        env: &Environment,
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        let inner = cell_style(env.kind, style);
        let cells: Vec<Vec<Layout>> = env
            .rows
            .iter()
            .map(|row| {
                let laid = row.iter().enumerate();
                laid.map(|(j, c)| self.cell(env.kind, j, c, inner, alpha))
                    .collect()
            })
            .collect();
        let table = self.grid(env, cells, inner, style);
        let s = style.scale(self.c());
        let (open, close) = fence_chars(env.kind);
        let reach = (table.ascent + table.descent) / 2.0;
        let target = (2.0 * reach * 0.901).max(2.0 * reach - 0.5 * s);
        let mut out = Layout::default();
        if let Some(ch) = open {
            out.push(self.delimiter(ch, target, s));
            out.space(FENCE_PAD * s);
        }
        out.push(table);
        if let Some(ch) = close {
            out.space(FENCE_PAD * s);
            out.push(self.delimiter(ch, target, s));
        }
        out
    }

    /// One cell. In `aligned` the cells after an `&` start with an empty atom,
    /// as amsmath does, so a leading `=` is spaced as a relation.
    fn cell(
        &self,
        kind: Env,
        col: usize,
        nodes: &[Node],
        style: Style,
        alpha: Option<Font>,
    ) -> Layout {
        if kind == Env::Aligned && col % 2 == 1 {
            let mut padded = Vec::with_capacity(nodes.len() + 1);
            padded.push(Node::Group(Vec::new()));
            padded.extend_from_slice(nodes);
            return self.list(&padded, style, alpha);
        }
        self.list(nodes, style, alpha)
    }

    /// The cells in rows and columns, centred on the axis of `outer`.
    fn grid(
        &self,
        env: &Environment,
        cells: Vec<Vec<Layout>>,
        inner: Style,
        outer: Style,
    ) -> Layout {
        let c = self.c();
        let columns = cells.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return Layout::default();
        }
        let plan = plan(env, columns);
        let cols = column_spans(&plan, &cells, columns, inner.scale(c));
        let rows = self.row_positions(env, &cells, inner);
        let height = rows.last().map_or(0.0, |r| r.bottom);
        let top = -(c.axis_height * outer.scale(c) + height / 2.0);
        let mut out = Layout::default();
        for (i, row) in cells.into_iter().enumerate() {
            for (j, cell) in row.into_iter().enumerate() {
                let slack = cols.widths[j] - cell.width;
                let dx = match plan.aligns[j] {
                    Col::Left | Col::Rule => 0.0,
                    Col::Right => slack,
                    Col::Center => slack / 2.0,
                };
                out.place(cell, cols.xs[j] + dx, top + rows[i].baseline);
            }
        }
        self.table_rules(&mut out, env, &plan, (&cols, &rows), (top, height), inner);
        // The rows' boxes, not just the ink: fences and neighbours size to them.
        out.ascent = out.ascent.max(-top);
        out.descent = out.descent.max(top + height);
        out.width = cols.total;
        out
    }

    /// `\hline`s across the table and the vertical rules of an `array` spec.
    fn table_rules(
        &self,
        out: &mut Layout,
        env: &Environment,
        plan: &Plan,
        (cols, rows): (&Columns, &[RowPos]),
        (top, height): (f32, f32),
        inner: Style,
    ) {
        let cs = inner.scale(self.c());
        let line = self.c().frac_rule * cs;
        for &i in &env.hlines {
            let y = rows.get(i).map_or(height - line, |r| r.top);
            out.rule(0.0, top + y, cols.total, line);
        }
        // Start below the air above a leading `\hline`, so the rules meet it.
        let from = rows.first().filter(|_| env.hlines.contains(&0));
        let from = from.map_or(0.0, |r| r.top);
        for &b in &plan.rules {
            let x = if b == 0 {
                0.0
            } else if b >= cols.xs.len() {
                cols.total - line
            } else {
                cols.xs[b] - plan.gaps[b] * cs / 2.0 - line / 2.0
            };
            out.rule(x, top + from, line, height - from);
        }
    }
}

/// Where the columns start and how wide they are.
struct Columns {
    xs: Vec<f32>,
    widths: Vec<f32>,
    total: f32,
}

fn column_spans(plan: &Plan, cells: &[Vec<Layout>], columns: usize, cs: f32) -> Columns {
    let widths: Vec<f32> = (0..columns)
        .map(|j| {
            let widest = cells.iter().filter_map(|r| r.get(j)).map(|l| l.width);
            widest.fold(0.0, f32::max)
        })
        .collect();
    let edge = |at: usize| {
        if plan.rules.contains(&at) {
            EDGE_RULE_PAD * cs
        } else {
            0.0
        }
    };
    let mut xs = Vec::with_capacity(columns);
    let mut at = edge(0);
    for (j, w) in widths.iter().enumerate() {
        at += plan.gaps[j] * cs;
        xs.push(at);
        at += w;
    }
    Columns {
        xs,
        widths,
        total: at + edge(columns),
    }
}

/// Where a row sits in the table, down from the table's top edge.
struct RowPos {
    top: f32,
    baseline: f32,
    bottom: f32,
}

impl Typesetter<'_> {
    /// Row positions: each as tall as its tallest cell (at least the strut),
    /// with room for a rule above it where `\hline` asks for one.
    fn row_positions(&self, env: &Environment, cells: &[Vec<Layout>], inner: Style) -> Vec<RowPos> {
        let c = self.c();
        let cs = inner.scale(c);
        let line = c.frac_rule * cs;
        let jot = if matches!(env.kind, Env::Aligned | Env::Gathered) {
            JOT * cs
        } else {
            0.0
        };
        let mut rows = Vec::with_capacity(cells.len());
        let mut y = 0.0;
        for (i, row) in cells.iter().enumerate() {
            if env.hlines.contains(&i) {
                y += line + 0.1 * cs;
            }
            let ascent = row
                .iter()
                .map(|l| l.ascent)
                .fold(STRUT_ASCENT * cs, f32::max);
            let descent = row
                .iter()
                .map(|l| l.descent)
                .fold(STRUT_DESCENT * cs, f32::max);
            rows.push(RowPos {
                top: y - line - 0.05 * cs,
                baseline: y + ascent,
                bottom: y + ascent + descent,
            });
            y += ascent + descent + jot;
        }
        if env.hlines.contains(&cells.len()) {
            y += line + 0.1 * cs;
        }
        if let Some(last) = rows.last_mut() {
            last.bottom = y - jot;
        }
        rows
    }
}
