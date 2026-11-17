use super::*;

const ROLES: &str = include_str!("../../assets/themes/roles.toml");
const FORMAT_MD: &str = include_str!("../../assets/themes/FORMAT.md");
const SHIPPED: &[(&str, &str)] = &[
    (
        "jade-light",
        include_str!("../../assets/themes/jade-light.toml"),
    ),
    (
        "latte",
        include_str!("../../assets/themes/catppuccin-latte.toml"),
    ),
    (
        "frappe",
        include_str!("../../assets/themes/catppuccin-frappe.toml"),
    ),
    (
        "macchiato",
        include_str!("../../assets/themes/catppuccin-macchiato.toml"),
    ),
    (
        "mocha",
        include_str!("../../assets/themes/catppuccin-mocha.toml"),
    ),
];

fn defaults() -> Defaults {
    Defaults::parse(ROLES).expect("roles.toml is valid")
}

fn load(name: &str) -> ResolvedTheme {
    let src = SHIPPED
        .iter()
        .find(|(n, _)| *n == name)
        .expect("known theme")
        .1;
    ResolvedTheme::parse(&defaults(), src).expect("shipped theme is valid")
}

fn rgb(hex: &str) -> Rgb {
    let v = u32::from_str_radix(hex, 16).expect("hex");
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

/// A minimal valid theme; tests mutate a copy to break one thing.
const MINI_PALETTE: &str = r##"
base = "#ffffff"
mantle = "#eeeeee"
crust = "#dddddd"
surface0 = "#cccccc"
surface1 = "#bbbbbb"
surface2 = "#aaaaaa"
overlay0 = "#999999"
overlay1 = "#888888"
overlay2 = "#777777"
subtext0 = "#666666"
text = "#000000"
lavender = "#7287fd"
rosewater = "#dc8a78"
blue = "#1e66f5"
sapphire = "#209fb5"
green = "#40a02b"
mauve = "#8839ef"
teal = "#179299"
yellow = "#df8e1d"
red = "#d20f39"
"##;

fn mini(extra: &str) -> String {
    format!(
        "format_version = 1\nname = \"Mini\"\nkind = \"light\"\n{extra}\n[palette]\n{MINI_PALETTE}"
    )
}

fn errors_of(src: &str) -> Vec<ThemeError> {
    ResolvedTheme::parse(&defaults(), src).expect_err("should be rejected")
}

#[test]
fn every_shipped_theme_loads() {
    for (name, _) in SHIPPED {
        let t = load(name);
        assert!(!t.name.is_empty(), "{name}");
    }
}

#[test]
fn resolves_palette_overrides_and_alphas() {
    let mocha = load("mocha");
    assert_eq!(mocha.kind, Kind::Dark);
    assert_eq!(mocha.color(Role::Background), rgb("1e1e2e"));
    assert_eq!(mocha.alpha(Alpha::Selection), 0.25);

    let latte = load("latte");
    assert_eq!(latte.color(Role::Link), rgb("1761f5"), "override wins");
    assert_eq!(latte.color(Role::Error), rgb("d20f39"), "default kept");

    let jade = load("jade-light");
    assert_eq!(jade.color(Role::Text), rgb("090909"));
    assert_eq!(jade.color(Role::Scrim), rgb("000000"));
    assert_eq!(jade.alpha(Alpha::Selection), 0.35);
    assert_eq!(jade.alpha(Alpha::Overlay), 0.5, "default kept");
}

#[test]
fn contrast_warnings_match_the_documented_state() {
    assert!(load("latte").warnings.is_empty());
    let jade = load("jade-light");
    assert!(jade.warnings.iter().any(|w| w.foreground == Role::Link));
    assert!(
        load("mocha")
            .warnings
            .iter()
            .all(|w| w.foreground == Role::TextSubtle)
    );
}

#[test]
fn every_role_is_documented_in_format_md() {
    for role in Role::ALL {
        assert!(
            FORMAT_MD.contains(&format!("| `{}` |", role.name())),
            "{}",
            role.name()
        );
    }
    for alpha in Alpha::ALL {
        assert!(
            FORMAT_MD.contains(&format!("`{}`", alpha.name())),
            "{}",
            alpha.name()
        );
    }
}

#[test]
fn mini_theme_is_valid() {
    ResolvedTheme::parse(&defaults(), &mini("")).expect("valid");
}

#[test]
fn typo_in_override_role_is_rejected() {
    let src = mini("") + "\n[overrides]\nwikilnk = \"blue\"\n";
    assert_eq!(
        errors_of(&src),
        vec![ThemeError::UnknownRole {
            section: "overrides",
            name: "wikilnk".into()
        }]
    );
}

#[test]
fn typo_in_top_level_or_section_key_is_rejected() {
    let src = mini("").replace("name =", "nmae =");
    assert!(matches!(errors_of(&src)[0], ThemeError::Syntax(_)));
    let src = mini("") + "\n[roles]\ntext = \"text\"\n";
    assert!(matches!(errors_of(&src)[0], ThemeError::Syntax(_)));
}

#[test]
fn unknown_and_out_of_range_alpha() {
    let src = mini("") + "\n[alpha]\nselektion = 0.2\nselection = 1.5\n";
    let errors = errors_of(&src);
    assert!(errors.contains(&ThemeError::UnknownAlpha {
        name: "selektion".into()
    }));
    assert!(errors.contains(&ThemeError::BadAlpha {
        name: "selection".into(),
        value: 1.5
    }));
}

#[test]
fn bad_hex_is_rejected_everywhere() {
    for bad in ["#fff", "ffffff", "#gggggg", "#ffffff00", "#ff ff f"] {
        let src = mini("").replace("base = \"#ffffff\"", &format!("base = \"{bad}\""));
        assert!(
            errors_of(&src)
                .iter()
                .any(|e| matches!(e, ThemeError::BadHex { .. })),
            "{bad}"
        );
    }
    let src = mini("") + "\n[overrides]\nlink = \"#12345\"\n";
    assert!(
        errors_of(&src)
            .iter()
            .any(|e| matches!(e, ThemeError::BadHex { .. }))
    );
}

#[test]
fn missing_palette_name_is_reported_per_role() {
    let src = mini("").replace("lavender = \"#7287fd\"\n", "");
    assert_eq!(
        errors_of(&src),
        vec![ThemeError::UnknownPaletteName {
            role: Role::Accent,
            name: "lavender".into()
        }]
    );
    // An override makes the default's palette name unnecessary.
    let src = src + "\n[overrides]\naccent = \"#7287fd\"\n";
    ResolvedTheme::parse(&defaults(), &src).expect("override covers the gap");
}

#[test]
fn all_problems_are_collected() {
    let src = mini("")
        .replace("lavender = \"#7287fd\"\n", "")
        .replace("format_version = 1", "format_version = 9")
        + "\n[overrides]\nwikilnk = \"blue\"\n";
    assert_eq!(errors_of(&src).len(), 3);
}

#[test]
fn name_and_version_rules() {
    let src = mini("").replace("name = \"Mini\"", "name = \"  \"");
    assert_eq!(errors_of(&src), vec![ThemeError::EmptyName]);
    let src = mini("").replace("name = \"Mini\"", &format!("name = \"{}\"", "x".repeat(65)));
    assert_eq!(errors_of(&src), vec![ThemeError::NameTooLong]);
    let src = mini("").replace("format_version = 1", "format_version = 2");
    assert_eq!(
        errors_of(&src),
        vec![ThemeError::FormatVersion { found: 2 }]
    );
}

#[test]
fn kind_must_be_light_or_dark() {
    let src = mini("").replace("\"light\"", "\"sepia\"");
    assert!(matches!(errors_of(&src)[0], ThemeError::Syntax(_)));
}

#[test]
fn size_and_palette_limits() {
    let big = format!("{}\n# {}", mini(""), "x".repeat(MAX_THEME_BYTES));
    assert!(matches!(errors_of(&big)[0], ThemeError::TooLarge { .. }));
    let mut many = mini("");
    for i in 0..=MAX_PALETTE_ENTRIES {
        many.push_str(&format!("extra{i} = \"#000000\"\n"));
    }
    assert!(
        errors_of(&many)
            .iter()
            .any(|e| matches!(e, ThemeError::TooManyPaletteEntries { .. }))
    );
}

#[test]
fn garbage_input_does_not_panic() {
    for src in [
        "",
        "not toml {{{",
        "\0\0\0",
        "[palette]\n",
        "name = 5",
        &"[".repeat(10_000),
    ] {
        assert!(ResolvedTheme::parse(&defaults(), src).is_err());
    }
}

#[test]
fn broken_defaults_are_rejected() {
    let missing_role = ROLES.replace("scrim                = \"crust\"", "");
    let errors = Defaults::parse(&missing_role).expect_err("missing role");
    assert!(errors.contains(&ThemeError::MissingDefault { name: "scrim" }));
    let missing_alpha = ROLES.replace("overlay            = 0.50", "");
    let errors = Defaults::parse(&missing_alpha).expect_err("missing alpha");
    assert!(errors.contains(&ThemeError::MissingDefault { name: "overlay" }));
    assert!(Defaults::parse("").is_err());
}

#[test]
fn load_time_is_small() {
    let defaults = defaults();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        for (_, src) in SHIPPED {
            ResolvedTheme::parse(&defaults, src).expect("valid");
        }
    }
    println!(
        "THEME parse: {:?} per theme (debug build)",
        start.elapsed() / 500
    );
}
