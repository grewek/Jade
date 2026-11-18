//! The fixed set of roles and alphas every theme defines.

macro_rules! named_enum {
    ($ty:ident { $($variant:ident => $name:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $ty { $($variant),+ }

        impl $ty {
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];
            pub const COUNT: usize = Self::ALL.len();

            /// The key used in theme files.
            pub fn name(self) -> &'static str {
                match self { $($ty::$variant => $name),+ }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.name() == name)
            }

            pub fn index(self) -> usize {
                self as usize
            }
        }
    };
}

named_enum!(Role {
    Background => "background",
    BackgroundSecondary => "background_secondary",
    BackgroundTertiary => "background_tertiary",
    Scrim => "scrim",
    Surface => "surface",
    SurfaceHover => "surface_hover",
    SurfaceActive => "surface_active",
    Border => "border",
    BorderStrong => "border_strong",
    TableHeader => "table_header",
    Text => "text",
    TextMuted => "text_muted",
    TextSubtle => "text_subtle",
    TextOnAccent => "text_on_accent",
    Accent => "accent",
    Cursor => "cursor",
    Selection => "selection",
    Link => "link",
    Wikilink => "wikilink",
    Code => "code",
    Info => "info",
    Summary => "summary",
    Tip => "tip",
    Success => "success",
    Warning => "warning",
    Error => "error",
    Neutral => "neutral",
});

named_enum!(Alpha {
    CalloutBackground => "callout_background",
    Selection => "selection",
    AccentHover => "accent_hover",
    AccentActive => "accent_active",
    Overlay => "overlay",
});
