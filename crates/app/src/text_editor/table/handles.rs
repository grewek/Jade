//! The controls on a rendered table's edge: a `+` circle at the column and row
//! boundary nearest the pointer, and a grip on the column and body row under
//! it to drag-reorder. Pure geometry in pixels relative to the table's top
//! left, so what is drawn and what a click hits come from the same numbers.

/// Row and column boundaries of one table box, in pixels from its top left.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::text_editor) struct Grid {
    /// `columns + 1` x positions: the left edge, each divider, the right edge.
    pub col_x: Vec<f32>,
    /// `rows + 1` y positions, row 0 being the header (with the delimiter's
    /// band under it): the top, then the bottom of every row.
    pub row_y: Vec<f32>,
}

impl Grid {
    fn columns(&self) -> usize {
        self.col_x.len() - 1
    }

    /// Rows including the header.
    fn rows(&self) -> usize {
        self.row_y.len() - 1
    }

    fn width(&self) -> f32 {
        self.col_x[self.columns()]
    }

    fn height(&self) -> f32 {
        self.row_y[self.rows()]
    }
}

/// A control. Boundary `b` of columns is the left edge of column `b` (`columns`
/// being the right edge); of rows it is the top of row `b` (`rows` being the
/// bottom), and the one above the header is never offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::text_editor) enum Handle {
    AddColumn(usize),
    AddRow(usize),
    ColumnGrip(usize),
    /// A body row (never the header).
    RowGrip(usize),
}

/// A grip being dragged: which table, which grip, and the boundary the pointer
/// is over now (the drop target).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::text_editor) struct TableDrag {
    pub table: usize,
    pub grip: Handle,
    pub over: usize,
}

/// Radius of a `+` circle.
pub(in crate::text_editor) const RADIUS: f32 = 8.0;
/// A grip's long and short side.
pub(in crate::text_editor) const GRIP_LONG: f32 = 22.0;
pub(in crate::text_editor) const GRIP_SHORT: f32 = 10.0;
/// How far left of and above the box the pointer still counts as over it, so
/// the controls straddling its edge can be reached.
pub(in crate::text_editor) const MARGIN: f32 = 16.0;

/// The controls to show with the pointer at `p`; none away from the table.
pub(in crate::text_editor) fn hover_at(grid: &Grid, p: (f32, f32)) -> Vec<Handle> {
    let (x, y) = p;
    if x < -MARGIN || y < -MARGIN || x > grid.width() + MARGIN || y > grid.height() + MARGIN {
        return Vec::new();
    }
    let mut shown = vec![
        Handle::AddColumn(nearest(&grid.col_x, x, 0)),
        Handle::ColumnGrip(span_at(&grid.col_x, x)),
        Handle::AddRow(nearest(&grid.row_y, y, 1)),
    ];
    let row = span_at(&grid.row_y, y);
    if row > 0 {
        shown.push(Handle::RowGrip(row));
    }
    shown
}

/// The boundary of `edges` closest to `at`, no lower than `first`.
fn nearest(edges: &[f32], at: f32, first: usize) -> usize {
    (first..edges.len())
        .min_by(|&a, &b| (edges[a] - at).abs().total_cmp(&(edges[b] - at).abs()))
        .unwrap_or(first)
}

/// The span of `edges` holding `at`, clamped to the first and last.
fn span_at(edges: &[f32], at: f32) -> usize {
    let spans = edges.len() - 1;
    (0..spans).find(|&i| at < edges[i + 1]).unwrap_or(spans - 1)
}

/// A control's rectangle as `(x, y, width, height)`.
pub(in crate::text_editor) fn rect_of(grid: &Grid, handle: Handle) -> (f32, f32, f32, f32) {
    let centered = |cx: f32, cy: f32, w: f32, h: f32| (cx - w / 2.0, cy - h / 2.0, w, h);
    match handle {
        Handle::AddColumn(b) => centered(grid.col_x[b], 0.0, 2.0 * RADIUS, 2.0 * RADIUS),
        Handle::AddRow(b) => centered(0.0, grid.row_y[b], 2.0 * RADIUS, 2.0 * RADIUS),
        Handle::ColumnGrip(c) => {
            let middle = (grid.col_x[c] + grid.col_x[c + 1]) / 2.0;
            centered(middle, 0.0, GRIP_LONG, GRIP_SHORT)
        }
        Handle::RowGrip(r) => {
            let middle = (grid.row_y[r] + grid.row_y[r + 1]) / 2.0;
            centered(0.0, middle, GRIP_SHORT, GRIP_LONG)
        }
    }
}

/// Which of the `shown` controls the point is on; the `+` circles win over
/// the grips where they overlap.
pub(in crate::text_editor) fn handle_at(
    grid: &Grid,
    shown: &[Handle],
    p: (f32, f32),
) -> Option<Handle> {
    let inside = |handle: Handle| {
        let (x, y, w, h) = rect_of(grid, handle);
        (x..=x + w).contains(&p.0) && (y..=y + h).contains(&p.1)
    };
    let is_add = |h: &Handle| matches!(h, Handle::AddColumn(_) | Handle::AddRow(_));
    shown
        .iter()
        .copied()
        .filter(is_add)
        .chain(shown.iter().copied().filter(|h| !is_add(h)))
        .find(|&h| inside(h))
}

/// Where a dragged column is dropped with the pointer at `x`: a column boundary.
pub(in crate::text_editor) fn column_drop(grid: &Grid, x: f32) -> usize {
    nearest(&grid.col_x, x, 0)
}

/// Where a dragged body row is dropped with the pointer at `y`: a row boundary
/// below the header.
pub(in crate::text_editor) fn row_drop(grid: &Grid, y: f32) -> usize {
    nearest(&grid.row_y, y, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two columns 100 and 60 wide; a header 40 tall and two body rows of 24.
    fn grid() -> Grid {
        Grid {
            col_x: vec![0.0, 100.0, 160.0],
            row_y: vec![0.0, 40.0, 64.0, 88.0],
        }
    }

    #[test]
    fn nothing_is_shown_away_from_the_table() {
        assert!(hover_at(&grid(), (-20.0, 10.0)).is_empty());
        assert!(hover_at(&grid(), (50.0, -20.0)).is_empty());
        assert!(hover_at(&grid(), (200.0, 10.0)).is_empty());
        assert!(hover_at(&grid(), (50.0, 120.0)).is_empty());
    }

    #[test]
    fn the_margin_left_of_and_above_the_box_still_counts_as_over_it() {
        assert!(!hover_at(&grid(), (-10.0, 50.0)).is_empty());
        assert!(!hover_at(&grid(), (50.0, -10.0)).is_empty());
    }

    #[test]
    fn the_nearest_boundaries_and_the_spans_under_the_pointer_are_shown() {
        let shown = hover_at(&grid(), (90.0, 60.0));
        assert!(shown.contains(&Handle::AddColumn(1)));
        assert!(shown.contains(&Handle::ColumnGrip(0)));
        assert!(shown.contains(&Handle::AddRow(2)));
        assert!(shown.contains(&Handle::RowGrip(1)));
    }

    #[test]
    fn the_header_has_no_grip_and_no_row_goes_above_it() {
        let shown = hover_at(&grid(), (10.0, 5.0));
        assert!(!shown.iter().any(|h| matches!(h, Handle::RowGrip(_))));
        assert_eq!(shown[2], Handle::AddRow(1), "the nearest allowed boundary");
    }

    #[test]
    fn the_ends_have_boundaries_too() {
        let shown = hover_at(&grid(), (158.0, 86.0));
        assert!(shown.contains(&Handle::AddColumn(2)));
        assert!(shown.contains(&Handle::AddRow(3)));
    }

    #[test]
    fn a_click_hits_a_shown_control_and_a_circle_beats_a_grip() {
        let grid = grid();
        let shown = hover_at(&grid, (50.0, 52.0));
        // The column grip sits on the top edge at the column's middle.
        let (x, y, w, h) = rect_of(&grid, Handle::ColumnGrip(0));
        assert_eq!(
            handle_at(&grid, &shown, (x + w / 2.0, y + h / 2.0)),
            Some(Handle::ColumnGrip(0))
        );
        assert_eq!(handle_at(&grid, &shown, (50.0, 30.0)), None);
        // A circle over a grip takes the click.
        let both = [Handle::ColumnGrip(0), Handle::AddColumn(0)];
        assert_eq!(
            handle_at(&grid, &both, (0.0, 0.0)),
            Some(Handle::AddColumn(0))
        );
    }

    #[test]
    fn drops_snap_to_the_nearest_boundary() {
        let grid = grid();
        assert_eq!(column_drop(&grid, 120.0), 1);
        assert_eq!(column_drop(&grid, 150.0), 2);
        assert_eq!(row_drop(&grid, 5.0), 1, "never above the header");
        assert_eq!(row_drop(&grid, 80.0), 3);
    }
}
