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
    // Element styling (#58). Each has a `fallback`, so roles.toml need not name it.
    Heading => "heading",
    Heading1 => "heading_1",
    Heading2 => "heading_2",
    Heading3 => "heading_3",
    Heading4 => "heading_4",
    Heading5 => "heading_5",
    Heading6 => "heading_6",
    QuoteBar => "quote_bar",
    LinkUnderline => "link_underline",
    CodeBackground => "code_background",
    Highlight => "highlight",
});

impl Role {
    /// The role this one takes its color from when neither the theme nor
    /// roles.toml assigns it, so the look is unchanged until a theme (or the
    /// user's `[colors]`) says otherwise. `None`: roles.toml must assign it.
    pub fn fallback(self) -> Option<Role> {
        match self {
            Role::Heading => Some(Role::Text),
            Role::Heading1
            | Role::Heading2
            | Role::Heading3
            | Role::Heading4
            | Role::Heading5
            | Role::Heading6 => Some(Role::Heading),
            Role::QuoteBar => Some(Role::BorderStrong),
            Role::LinkUnderline => Some(Role::Link),
            Role::CodeBackground => Some(Role::Surface),
            _ => None,
        }
    }

    /// `heading_1`..`heading_6` for level 1..=6 (anything else is clamped).
    pub fn heading_level(level: u8) -> Role {
        const LEVELS: [Role; 6] = [
            Role::Heading1,
            Role::Heading2,
            Role::Heading3,
            Role::Heading4,
            Role::Heading5,
            Role::Heading6,
        ];
        LEVELS[usize::from(level.clamp(1, 6)) - 1]
    }
}

named_enum!(Alpha {
    CalloutBackground => "callout_background",
    Selection => "selection",
    AccentHover => "accent_hover",
    AccentActive => "accent_active",
    Overlay => "overlay",
});
