//! The element tables of `config.toml`: `[inline_code]`, `[code_block]`,
//! `[callout]`, `[table]`, `[image]`, `[link]`, `[blockquote]`, `[list]`,
//! `[task]` and `[spacing]`. Same rules as the rest of the reader.

use toml::{Table, Value};

use super::{Range, Reader};
use crate::settings::typography::{BulletShape, DoneStyle, Radius, Typography, Underline};

const EM: Range = (0.0, 4.0);
const BAR: Range = (0.0, 16.0);
const INDENT: Range = (0.0, 8.0);
/// A code row is a body row, so code can shrink but not grow.
const CODE_SCALE: Range = (0.5, 1.0);
const GAP: Range = (0.0, 8.0);

impl Reader {
    pub(super) fn blocks(&mut self, root: &Table, t: &mut Typography) {
        self.inline_code(root, t);
        self.code_block(root, t);
        self.callout_table_image(root, t);
        self.link_quote_list(root, t);
        self.task_spacing(root, t);
    }

    fn boolean(&mut self, (table, name): (&Table, &str), key: &str, default: bool) -> bool {
        match table.get(key).map(Value::as_bool) {
            None => default,
            Some(Some(b)) => b,
            Some(None) => {
                self.notice(format!(
                    "{name}.{key} must be true or false, using {default}"
                ));
                default
            }
        }
    }

    /// One of `names` (case-insensitive); `default` and a notice otherwise.
    fn choice<T: Copy>(
        &mut self,
        (table, name): (&Table, &str),
        key: &str,
        names: &[(&str, T)],
        default: T,
    ) -> T {
        let Some(value) = table.get(key) else {
            return default;
        };
        let found = value
            .as_str()
            .and_then(|s| names.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)));
        match found {
            Some(&(_, v)) => v,
            None => {
                let all: Vec<&str> = names.iter().map(|(n, _)| *n).collect();
                self.notice(format!(
                    "{name}.{key} must be one of {all:?}, using the default"
                ));
                default
            }
        }
    }

    /// An element's own radius: kept as it is when the key is absent.
    fn own_radius(
        &mut self,
        (table, name): (&Table, &str),
        default: Option<Radius>,
    ) -> Option<Radius> {
        let Some(value) = table.get("radius") else {
            return default;
        };
        self.try_radius(&format!("{name}.radius"), value)
            .or(default)
    }

    fn inline_code(&mut self, root: &Table, t: &mut Typography) {
        let known = ["background", "padding", "radius"];
        let Some(table) = self.section(root, "inline_code", &known) else {
            return;
        };
        let at = (table, "inline_code");
        let c = &mut t.inline_code;
        c.background = self.boolean(at, "background", c.background);
        c.padding = self.number(at, "padding", EM, c.padding);
        c.radius = self.own_radius(at, c.radius);
    }

    fn code_block(&mut self, root: &Table, t: &mut Typography) {
        let known = [
            "background",
            "padding",
            "border",
            "radius",
            "language_label",
            "font_scale",
        ];
        let Some(table) = self.section(root, "code_block", &known) else {
            return;
        };
        let at = (table, "code_block");
        let c = &mut t.code_block;
        c.background = self.boolean(at, "background", c.background);
        c.padding = self.number(at, "padding", EM, c.padding);
        c.border = self.boolean(at, "border", c.border);
        c.radius = self.own_radius(at, c.radius);
        c.language_label = self.boolean(at, "language_label", c.language_label);
        c.font_scale = self.number(at, "font_scale", CODE_SCALE, c.font_scale);
    }

    fn callout_table_image(&mut self, root: &Table, t: &mut Typography) {
        if let Some(table) = self.section(root, "callout", &["radius", "bar_width"]) {
            let at = (table, "callout");
            t.callout.radius = self.own_radius(at, t.callout.radius);
            t.callout.bar_width = self.number(at, "bar_width", BAR, t.callout.bar_width);
        }
        let known = ["radius", "cell_padding", "zebra", "header_bold"];
        if let Some(table) = self.section(root, "table", &known) {
            let at = (table, "table");
            let c = &mut t.table;
            c.radius = self.own_radius(at, c.radius);
            c.cell_padding = self.number(at, "cell_padding", EM, c.cell_padding);
            c.zebra = self.boolean(at, "zebra", c.zebra);
            c.header_bold = self.boolean(at, "header_bold", c.header_bold);
        }
        if let Some(table) = self.section(root, "image", &["radius"]) {
            t.image.radius = self.own_radius((table, "image"), t.image.radius);
        }
    }

    fn link_quote_list(&mut self, root: &Table, t: &mut Typography) {
        let modes = [
            ("always", Underline::Always),
            ("hover", Underline::Hover),
            ("never", Underline::Never),
        ];
        if let Some(table) = self.section(root, "link", &["underline", "wikilink_underline"]) {
            let at = (table, "link");
            t.link.underline = self.choice(at, "underline", &modes, t.link.underline);
            let wiki = t.link.wikilink_underline;
            t.link.wikilink_underline = self.choice(at, "wikilink_underline", &modes, wiki);
        }
        if let Some(table) = self.section(root, "blockquote", &["italic", "bar_width", "indent"]) {
            let at = (table, "blockquote");
            let q = &mut t.blockquote;
            q.italic = self.boolean(at, "italic", q.italic);
            q.bar_width = self.number(at, "bar_width", BAR, q.bar_width);
            q.indent = self.number(at, "indent", INDENT, q.indent);
        }
        let known = ["indent", "item_gap", "bullet_shapes", "number_style"];
        if let Some(table) = self.section(root, "list", &known) {
            self.list(table, t);
        }
    }

    fn list(&mut self, table: &Table, t: &mut Typography) {
        let at = (table, "list");
        t.list.indent = self.number(at, "indent", INDENT, t.list.indent);
        t.list.item_gap = self.number(at, "item_gap", GAP, t.list.item_gap);
        // Decimal is the only numbering there is.
        self.choice(at, "number_style", &[("decimal", ())], ());
        let Some(value) = table.get("bullet_shapes") else {
            return;
        };
        let Some(items) = value.as_array() else {
            self.notice(
                "list.bullet_shapes must be an array of shape names, using the default".into(),
            );
            return;
        };
        let shapes = [
            ("disc", BulletShape::Disc),
            ("ring", BulletShape::Ring),
            ("square", BulletShape::Square),
        ];
        let default = t.list.bullet_shapes;
        t.list.bullet_shapes = self.spread("list.bullet_shapes", items, default, |_, v| {
            let name = v.as_str()?;
            shapes
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|&(_, s)| s)
        });
    }

    fn task_spacing(&mut self, root: &Table, t: &mut Typography) {
        if let Some(table) = self.section(root, "task", &["done_style", "checkbox_radius"]) {
            let at = (table, "task");
            let styles = [
                ("dim", DoneStyle::Dim),
                ("strike", DoneStyle::Strike),
                ("both", DoneStyle::Both),
                ("none", DoneStyle::None),
            ];
            t.task.done_style = self.choice(at, "done_style", &styles, t.task.done_style);
            if let Some(v) = table.get("checkbox_radius") {
                t.task.checkbox_radius =
                    self.radius("task.checkbox_radius", v, t.task.checkbox_radius);
            }
        }
        if let Some(table) = self.section(root, "strikethrough", &["muted"]) {
            let muted = t.strikethrough.muted;
            t.strikethrough.muted = self.boolean((table, "strikethrough"), "muted", muted);
        }
        if let Some(table) = self.section(root, "spacing", &["block_gap"]) {
            let gap = t.spacing.block_gap;
            t.spacing.block_gap = self.number((table, "spacing"), "block_gap", GAP, gap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::typography::Boxed;

    fn read(src: &str) -> crate::settings::Read {
        Typography::from_config(src)
    }

    #[test]
    fn every_element_table_is_read() {
        let r = read(
            "[inline_code]\nbackground = true\npadding = 0.2\nradius = 4\n\
             [code_block]\nbackground = true\npadding = 0.6\nborder = true\nradius = \"full\"\n\
             language_label = true\nfont_scale = 0.9\n\
             [callout]\nradius = 8\nbar_width = 0\n\
             [table]\nradius = 2\ncell_padding = 0.5\nzebra = true\nheader_bold = false\n\
             [image]\nradius = 6\n\
             [link]\nunderline = \"Hover\"\nwikilink_underline = \"always\"\n\
             [blockquote]\nitalic = false\nbar_width = 3\nindent = 1.5\n\
             [list]\nindent = 2\nitem_gap = 0.25\nbullet_shapes = [\"disc\", \"ring\"]\nnumber_style = \"decimal\"\n\
             [task]\ndone_style = \"both\"\ncheckbox_radius = 5\n\
             [strikethrough]\nmuted = true\n\
             [spacing]\nblock_gap = 1.5\n",
        );
        assert_eq!(r.notices, Vec::<String>::new());
        let t = r.typography;
        assert!(t.inline_code.background && t.inline_code.padding == 0.2);
        assert_eq!(t.radius_of(Boxed::InlineCode), Radius::Px(4.0));
        assert_eq!(t.radius_of(Boxed::CodeBlock), Radius::Full);
        assert!(t.code_block.border && t.code_block.language_label);
        assert_eq!(t.code_block.font_scale, 0.9);
        assert_eq!(
            (t.callout.radius, t.callout.bar_width),
            (Some(Radius::Px(8.0)), 0.0)
        );
        assert_eq!(t.radius_of(Boxed::Table), Radius::Px(2.0));
        assert!(t.table.zebra && !t.table.header_bold);
        assert_eq!(t.radius_of(Boxed::Image), Radius::Px(6.0));
        assert_eq!(t.link.underline, Underline::Hover);
        assert_eq!(t.link.wikilink_underline, Underline::Always);
        assert!(!t.blockquote.italic);
        assert_eq!((t.blockquote.bar_width, t.blockquote.indent), (3.0, 1.5));
        assert_eq!((t.list.indent, t.list.item_gap), (2.0, 0.25));
        // Short arrays repeat their last value.
        assert_eq!(
            t.list.bullet_shapes,
            [BulletShape::Disc, BulletShape::Ring, BulletShape::Ring]
        );
        assert!(t.strikethrough.muted);
        assert_eq!(t.task.done_style, DoneStyle::Both);
        assert_eq!(t.task.checkbox_radius, Radius::Px(5.0));
        assert_eq!(t.spacing.block_gap, 1.5);
    }

    #[test]
    fn a_bad_value_keeps_that_key_and_notices_once() {
        let r = read(
            "[inline_code]\nbackground = \"yes\"\npadding = 99\nradius = true\n\
             [link]\nunderline = \"sometimes\"\n\
             [table]\nradius = []\nzebra = 1\n\
             [list]\nbullet_shapes = [\"star\"]\nnumber_style = \"roman\"\n\
             [task]\ndone_style = 3\ncheckbox_radius = \"x\"\n",
        );
        let (t, d) = (r.typography, Typography::default());
        assert_eq!(t.inline_code.background, d.inline_code.background);
        assert_eq!(t.inline_code.padding, 4.0, "clamped");
        assert_eq!(t.inline_code.radius, d.inline_code.radius);
        assert_eq!(t.link.underline, d.link.underline);
        // The table's own default radius survives a bad value.
        assert_eq!(t.table.radius, d.table.radius);
        assert_eq!(t.table.zebra, d.table.zebra);
        assert_eq!(t.list.bullet_shapes, d.list.bullet_shapes);
        assert_eq!(t.task.done_style, d.task.done_style);
        assert_eq!(t.task.checkbox_radius, d.task.checkbox_radius);
        assert_eq!(r.notices.len(), 10, "{:#?}", r.notices);
    }

    #[test]
    fn unknown_keys_in_an_element_table_are_noticed() {
        let r = read("[callout]\nradious = 4\n[spacing]\nblock_gap = 2\nextra = 1\n");
        assert_eq!(r.typography.spacing.block_gap, 2.0);
        assert_eq!(r.notices.len(), 2, "{:?}", r.notices);
    }

    #[test]
    fn a_radius_in_an_element_wins_over_the_shape_radius() {
        let r = read("[shape]\nradius = 10\n[callout]\nradius = 2\n");
        let t = r.typography;
        assert_eq!(t.radius_of(Boxed::Callout), Radius::Px(2.0));
        assert_eq!(t.radius_of(Boxed::CodeBlock), Radius::Px(10.0));
        // The table keeps its own default, as it does today.
        assert_eq!(t.radius_of(Boxed::Table), Radius::Px(6.0));
    }
}
