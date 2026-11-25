//! The resolved typography settings: one plain value per `config.toml` key,
//! all of it `Copy`. [`Typography::default`] is today's look, so building the
//! struct changes nothing on screen; each later step switches a key on where
//! it implements it. Sizes are multiples of [`Font::size`] (em, or rows of the
//! line height); only radii, border and bar widths are px.

/// H1..H6.
pub const HEADING_LEVELS: usize = 6;

/// Only IBM Plex Mono is supported until the app is stable on it: the row
/// estimates and the formula columns assume its character width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FontFamily {
    #[default]
    IbmPlexMono,
}

/// A corner radius: px, or half the box's height (a pill).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Radius {
    Px(f32),
    Full,
}

impl Radius {
    /// The radius in px for a box `height` px tall.
    pub fn px(self, height: f32) -> f32 {
        match self {
            Radius::Px(r) => r.min(height / 2.0).max(0.0),
            Radius::Full => height / 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underline {
    Always,
    Hover,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoneStyle {
    Dim,
    Strike,
    Both,
    None,
}

/// Drawn shapes, never glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulletShape {
    Disc,
    Ring,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub family: FontFamily,
    pub code_family: FontFamily,
    /// px; the unit of every other size.
    pub size: f32,
    /// A multiple of `size`.
    pub line_height: f32,
    /// Formulas relative to the built-in formula size.
    pub math_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    /// For every block with a visible box that sets none of its own.
    pub radius: Radius,
    pub border_width: f32,
}

/// One value per level, index 0 = H1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heading {
    pub scale: [f32; HEADING_LEVELS],
    pub weight: [u16; HEADING_LEVELS],
    /// Text rows.
    pub space_above: [f32; HEADING_LEVELS],
    pub space_below: [f32; HEADING_LEVELS],
    /// A line under the heading.
    pub rule: [bool; HEADING_LEVELS],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineCode {
    pub background: bool,
    /// em, left and right.
    pub padding: f32,
    pub radius: Option<Radius>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CodeBlock {
    pub background: bool,
    /// em.
    pub padding: f32,
    pub border: bool,
    pub radius: Option<Radius>,
    pub language_label: bool,
    pub font_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Callout {
    pub radius: Option<Radius>,
    /// px; 0 = no accent bar.
    pub bar_width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Table {
    pub radius: Option<Radius>,
    /// em, on top of the character grid.
    pub cell_padding: f32,
    pub zebra: bool,
    pub header_bold: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub radius: Option<Radius>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub underline: Underline,
    pub wikilink_underline: Underline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blockquote {
    pub italic: bool,
    /// px.
    pub bar_width: f32,
    /// em per level.
    pub indent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct List {
    /// em per level.
    pub indent: f32,
    /// Rows between items.
    pub item_gap: f32,
    /// By depth; deeper levels repeat the last.
    pub bullet_shapes: [BulletShape; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    pub done_style: DoneStyle,
    pub checkbox_radius: Radius,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// Rows between blocks.
    pub block_gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub font: Font,
    pub shape: Shape,
    pub heading: Heading,
    pub inline_code: InlineCode,
    pub code_block: CodeBlock,
    pub callout: Callout,
    pub table: Table,
    pub image: Image,
    pub link: Link,
    pub blockquote: Blockquote,
    pub list: List,
    pub task: Task,
    pub spacing: Spacing,
}

impl Default for Typography {
    /// Today's look: the values the editor hard-codes now. Headings are
    /// plain bold at body size, nothing draws a box, links have no underline.
    fn default() -> Typography {
        Typography {
            font: Font {
                family: FontFamily::IbmPlexMono,
                code_family: FontFamily::IbmPlexMono,
                size: 16.0,
                line_height: 1.625,
                math_scale: 1.0,
            },
            shape: Shape {
                radius: Radius::Px(0.0),
                border_width: 1.0,
            },
            heading: Heading {
                scale: [1.0; HEADING_LEVELS],
                weight: [700; HEADING_LEVELS],
                space_above: [0.0; HEADING_LEVELS],
                space_below: [0.0; HEADING_LEVELS],
                rule: [false; HEADING_LEVELS],
            },
            inline_code: InlineCode {
                background: false,
                padding: 0.0,
                radius: None,
            },
            code_block: CodeBlock {
                background: false,
                padding: 0.0,
                border: false,
                radius: None,
                language_label: false,
                font_scale: 1.0,
            },
            callout: Callout {
                radius: None,
                bar_width: 3.0,
            },
            table: Table {
                radius: Some(Radius::Px(6.0)),
                cell_padding: 0.0,
                zebra: false,
                header_bold: true,
            },
            image: Image { radius: None },
            link: Link {
                underline: Underline::Never,
                wikilink_underline: Underline::Never,
            },
            blockquote: Blockquote {
                italic: true,
                bar_width: 2.0,
                indent: 1.25,
            },
            list: List {
                indent: 1.25,
                item_gap: 0.0,
                bullet_shapes: [BulletShape::Disc; 3],
            },
            task: Task {
                done_style: DoneStyle::None,
                checkbox_radius: Radius::Px(3.0),
            },
            spacing: Spacing { block_gap: 1.0 },
        }
    }
}

/// A block with a visible box, for [`Typography::radius_of`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boxed {
    InlineCode,
    CodeBlock,
    Callout,
    Table,
    Image,
}

impl Typography {
    /// The radius of `block`: its own, else `[shape] radius`.
    pub fn radius_of(&self, block: Boxed) -> Radius {
        let own = match block {
            Boxed::InlineCode => self.inline_code.radius,
            Boxed::CodeBlock => self.code_block.radius,
            Boxed::Callout => self.callout.radius,
            Boxed::Table => self.table.radius,
            Boxed::Image => self.image.radius,
        };
        own.unwrap_or(self.shape.radius)
    }

    /// The settings of heading `level` (1..=6; anything else is clamped, so a
    /// caller never indexes out of range).
    pub fn heading_index(level: u8) -> usize {
        usize::from(level.clamp(1, HEADING_LEVELS as u8)) - 1
    }

    /// Whether going from `self` to `other` changes where anything sits (sizes,
    /// spacing, padding) and not only how it is painted. A key is paint-only
    /// when it is listed in `paint_only_from`; a new key counts as
    /// layout until it is listed there, so a rebuild is never missed.
    pub fn layout_differs(&self, other: &Typography) -> bool {
        let mut same_paint = *self;
        same_paint.paint_only_from(other);
        same_paint != *other
    }

    /// Copies the keys that never move anything from `other`.
    fn paint_only_from(&mut self, other: &Typography) {
        self.shape = other.shape;
        self.heading.weight = other.heading.weight;
        self.heading.rule = other.heading.rule;
        self.inline_code.background = other.inline_code.background;
        self.inline_code.radius = other.inline_code.radius;
        self.inline_code.padding = other.inline_code.padding;
        self.code_block.border = other.code_block.border;
        self.code_block.padding = other.code_block.padding;
        self.code_block.radius = other.code_block.radius;
        self.callout.radius = other.callout.radius;
        self.table.radius = other.table.radius;
        self.table.zebra = other.table.zebra;
        self.table.header_bold = other.table.header_bold;
        self.image.radius = other.image.radius;
        self.link = other.link;
        self.blockquote.italic = other.blockquote.italic;
        self.list.bullet_shapes = other.list.bullet_shapes;
        self.task = other.task;
    }

    /// The line height in px.
    pub fn line_height_px(&self) -> f32 {
        self.font.size * self.font.line_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_todays_look() {
        let t = Typography::default();
        // 16px text on a 26px row: GPUI's default text style.
        assert_eq!(t.line_height_px(), 26.0);
        assert_eq!(t.heading.scale, [1.0; HEADING_LEVELS]);
        assert_eq!(t.heading.weight, [700; HEADING_LEVELS]);
        assert!(!t.heading.rule.iter().any(|&r| r));
        // The constants the editor still hard-codes.
        assert_eq!(t.radius_of(Boxed::Table), Radius::Px(6.0));
        assert_eq!(t.task.checkbox_radius, Radius::Px(3.0));
        assert_eq!(t.callout.bar_width, 3.0);
        assert_eq!(t.blockquote.bar_width, 2.0);
        // 20px at 16px text.
        assert_eq!(t.list.indent * t.font.size, 20.0);
        assert_eq!(t.blockquote.indent * t.font.size, 20.0);
    }

    #[test]
    fn a_block_without_a_radius_uses_the_shape_radius() {
        let mut t = Typography::default();
        assert_eq!(t.radius_of(Boxed::Callout), Radius::Px(0.0));
        t.shape.radius = Radius::Px(8.0);
        assert_eq!(t.radius_of(Boxed::Callout), Radius::Px(8.0));
        assert_eq!(t.radius_of(Boxed::Image), Radius::Px(8.0));
        // Its own wins; the table keeps its 6.
        assert_eq!(t.radius_of(Boxed::Table), Radius::Px(6.0));
        t.callout.radius = Some(Radius::Full);
        assert_eq!(t.radius_of(Boxed::Callout), Radius::Full);
    }

    #[test]
    fn a_radius_never_exceeds_half_the_box() {
        assert_eq!(Radius::Px(6.0).px(40.0), 6.0);
        assert_eq!(Radius::Px(60.0).px(40.0), 20.0);
        assert_eq!(Radius::Px(-3.0).px(40.0), 0.0);
        assert_eq!(Radius::Full.px(26.0), 13.0);
        assert_eq!(Radius::Full.px(0.0), 0.0);
    }

    #[test]
    fn only_a_layout_key_makes_the_layout_differ() {
        let base = Typography::default();
        let mut paint = base;
        paint.shape.radius = Radius::Full;
        paint.heading.weight = [400; HEADING_LEVELS];
        paint.link.underline = Underline::Always;
        paint.task.done_style = DoneStyle::Both;
        paint.table.zebra = true;
        assert!(!base.layout_differs(&paint));
        let edits: [fn(&mut Typography); 7] = [
            |t| t.font.size = 18.0,
            |t| t.heading.scale[2] = 1.5,
            |t| t.heading.space_above[0] = 1.0,
            |t| t.table.cell_padding = 0.5,
            |t| t.code_block.background = true,
            |t| t.list.indent = 2.0,
            |t| t.spacing.block_gap = 2.0,
        ];
        for edit in edits {
            let mut layout = base;
            edit(&mut layout);
            assert!(base.layout_differs(&layout));
            assert!(layout.layout_differs(&base));
        }
    }

    #[test]
    fn heading_levels_clamp_into_range() {
        assert_eq!(Typography::heading_index(1), 0);
        assert_eq!(Typography::heading_index(6), 5);
        assert_eq!(Typography::heading_index(0), 0);
        assert_eq!(Typography::heading_index(9), 5);
    }
}
