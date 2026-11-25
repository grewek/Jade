use super::*;

const ROLES: &str = include_str!("../../assets/themes/roles.toml");
const FORMAT_MD: &str = include_str!("../../assets/themes/FORMAT.md");
const SHIPPED: &[(&str, &str)] = &[
    (
        "jade-light",
        include_str!("../../assets/themes/jade-light.toml"),
    ),
    (
        "jade-neon",
        include_str!("../../assets/themes/jade-neon.toml"),
    ),
    (
        "jade-neon-light",
        include_str!("../../assets/themes/jade-neon-light.toml"),
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
fn jade_neon_is_dark_with_a_mint_accent() {
    let neon = load("jade-neon");
    assert_eq!(neon.kind, Kind::Dark);
    assert_eq!(neon.color(Role::Accent), rgb("3dffa2"));
    assert_eq!(neon.color(Role::Selection), rgb("3bae76"));
    assert_eq!(neon.color(Role::Error), rgb("ff5c7a"), "default kept");
    assert_eq!(neon.alpha(Alpha::Selection), 0.35);
    assert!(neon.warnings.is_empty(), "{:?}", neon.warnings);
}

#[test]
fn jade_neon_light_is_light_with_a_deep_mint_accent() {
    let neon = load("jade-neon-light");
    assert_eq!(neon.kind, Kind::Light);
    assert_eq!(neon.color(Role::Accent), rgb("087a4b"));
    assert_eq!(neon.color(Role::Selection), rgb("9be8ba"));
    assert_eq!(neon.alpha(Alpha::Selection), 0.55);
    assert!(neon.warnings.is_empty(), "{:?}", neon.warnings);
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
            roles: vec![Role::Accent],
            name: "lavender".into(),
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

mod engine_setup {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use gpui_kit::{AppContext as _, TestAppContext};
    use tempfile::{TempDir, tempdir};

    use crate::theme::builtin::{self, DEFAULT_ID};
    use crate::theme::choices;
    use crate::theme::loader::{load_checked, load_startup, resolve};
    use crate::theme::tests::mini;
    use crate::theme::{ConfigDirs, Role, ThemeEngine};

    fn dirs_in(tmp: &TempDir) -> ConfigDirs {
        let dirs = ConfigDirs::new(tmp.path().to_path_buf());
        fs::create_dir_all(dirs.theme_dir()).expect("theme dir");
        dirs
    }

    fn write(path: &Path, content: impl AsRef<[u8]>) {
        fs::write(path, content).expect("write");
    }

    fn user_theme(dirs: &ConfigDirs, id: &str, content: impl AsRef<[u8]>) {
        write(&dirs.theme_dir().join(format!("{id}.toml")), content);
    }

    #[test]
    fn config_dir_resolution() {
        let os = Some(PathBuf::from("/home/u/.config"));
        let over = Some(OsString::from("/tmp/x"));
        let resolve = ConfigDirs::resolve;
        assert_eq!(
            resolve(over.clone(), os.clone(), false),
            Some(ConfigDirs::new("/tmp/x".into()))
        );
        assert_eq!(
            resolve(over, os.clone(), true),
            Some(ConfigDirs::new("/tmp/x".into()))
        );
        assert_eq!(
            resolve(None, os.clone(), false),
            Some(ConfigDirs::new("/home/u/.config/jade".into()))
        );
        assert_eq!(
            resolve(Some(OsString::new()), os.clone(), false),
            Some(ConfigDirs::new("/home/u/.config/jade".into())),
            "an empty override is ignored"
        );
        assert_eq!(resolve(None, None, false), None);
        // A debug build never reaches the OS directory.
        let dev = resolve(None, os, true).expect("dev dir");
        assert!(
            dev.theme_dir().ends_with("target/jade-dev/theme"),
            "{:?}",
            dev.theme_dir()
        );
        assert!(!dev.theme_dir().starts_with("/home/u"));
    }

    #[test]
    fn dirs_layout() {
        let dirs = ConfigDirs::new("/c".into());
        assert_eq!(dirs.theme_dir(), PathBuf::from("/c/theme"));
        assert_eq!(dirs.config_file(), PathBuf::from("/c/config.toml"));
    }

    #[test]
    fn startup_defaults_without_any_config() {
        let defaults = builtin::defaults();
        let none = load_startup(&defaults, None);
        assert_eq!(none.id, DEFAULT_ID);
        assert!(
            none.notices.iter().all(|n| n.contains("warning")),
            "{:?}",
            none.notices
        );
        let tmp = tempdir().expect("tmp");
        let empty = load_startup(
            &defaults,
            Some(&ConfigDirs::new(tmp.path().join("missing"))),
        );
        assert_eq!(empty.id, DEFAULT_ID);
    }

    #[test]
    fn config_toml_picks_a_builtin_theme() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        write(
            &dirs.config_file(),
            "theme = \"catppuccin-mocha\"\nfuture_option = 3\n",
        );
        let loaded = load_startup(&builtin::defaults(), Some(&dirs));
        assert_eq!(loaded.id, "catppuccin-mocha");
        assert_eq!(loaded.theme.name, "Catppuccin Mocha");
    }

    #[test]
    fn user_file_shadows_a_builtin_and_adds_new_themes() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        let shipped = |name: &str| super::SHIPPED.iter().find(|(n, _)| *n == name).unwrap().1;
        let custom = shipped("mocha").replace("Catppuccin Mocha", "My Mocha");
        user_theme(&dirs, "catppuccin-mocha", &custom);
        user_theme(&dirs, "mine", shipped("latte"));
        let defaults = builtin::defaults();
        let theme_dir = dirs.theme_dir();
        let shadowed = resolve(&defaults, Some(&theme_dir), "catppuccin-mocha").expect("loads");
        assert_eq!(shadowed.name, "My Mocha");
        assert!(resolve(&defaults, Some(&theme_dir), "mine").is_ok());
        assert_eq!(
            resolve(&defaults, Some(&theme_dir), "jade-light")
                .expect("builtin")
                .name,
            "Jade Light"
        );
    }

    #[test]
    fn broken_theme_falls_back_with_messages() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        write(&dirs.config_file(), "theme = \"broken\"");
        user_theme(
            &dirs,
            "broken",
            super::mini("\n[overrides]\nwikilnk = \"blue\"\n"),
        );
        let loaded = load_startup(&builtin::defaults(), Some(&dirs));
        assert_eq!(loaded.id, DEFAULT_ID);
        assert!(
            loaded.notices.iter().any(|n| n.contains("wikilnk")),
            "{:?}",
            loaded.notices
        );
        assert!(loaded.notices.iter().any(|n| n.contains("built-in")));
    }

    #[test]
    fn bad_inputs_fall_back_without_panicking() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        let defaults = builtin::defaults();
        user_theme(&dirs, "binary", [0xff, 0xfe, 0x00, 0x80]);
        user_theme(&dirs, "huge", vec![b'#'; 200_000]);
        fs::create_dir(dirs.theme_dir().join("isdir.toml")).expect("dir");
        for (config, why) in [
            ("theme = \"binary\"", "UTF-8"),
            ("theme = \"huge\"", "limit"),
            ("theme = \"isdir\"", "not found"),
            ("theme = \"nope\"", "not found"),
            ("theme = \"../escape\"", "not allowed"),
            ("theme = \"\"", "not allowed"),
            ("theme = \"roles\"", "not allowed"),
            ("theme = 5", "config.toml"),
            ("not = [toml", "config.toml"),
        ] {
            write(&dirs.config_file(), config);
            let loaded = load_startup(&defaults, Some(&dirs));
            assert_eq!(loaded.id, DEFAULT_ID, "{config}");
            assert!(
                loaded.notices.iter().any(|n| n.contains(why)),
                "{config}: {:?}",
                loaded.notices
            );
        }
        write(&dirs.config_file(), [0xff, 0xff]);
        assert_eq!(load_startup(&defaults, Some(&dirs)).id, DEFAULT_ID);
    }

    #[test]
    fn user_roles_toml_is_never_a_theme() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        user_theme(&dirs, "roles", "garbage");
        assert!(resolve(&builtin::defaults(), Some(&dirs.theme_dir()), "roles").is_err());
    }

    #[gpui_kit::test]
    fn engine_installs_the_configured_theme(cx: &mut TestAppContext) {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        write(&dirs.config_file(), "theme = \"catppuccin-mocha\"");
        cx.update(|cx| {
            gpui_kit::init(cx);
            let notices = ThemeEngine::install(Some(dirs), cx);
            assert!(notices.iter().all(|n| n.contains("warning")), "{notices:?}");
            let engine = ThemeEngine::global(cx);
            assert_eq!(engine.active_id(), "catppuccin-mocha");
            let bg = engine.palette().color(Role::Background);
            let expected: gpui_kit::Hsla = gpui_kit::rgb(0x1e1e2e).into();
            assert_eq!(bg, expected);
            let _ = cx.new(|_| ());
        });
    }

    #[test]
    fn debounce_fires_once_after_a_burst_goes_quiet() {
        let mut d = super::super::watch::Debounce::default();
        assert!(!d.tick(0));
        assert!(!d.tick(3));
        assert!(!d.tick(1));
        assert!(d.tick(0));
        assert!(!d.tick(0));
    }

    #[test]
    fn load_checked_fails_instead_of_falling_back() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        write(&dirs.config_file(), "theme = \"nope\"");
        let errors = load_checked(&builtin::defaults(), Some(&dirs)).err();
        assert!(errors.is_some_and(|e| e.iter().any(|m| m.contains("not found"))));
        write(&dirs.config_file(), "theme = [");
        assert!(load_checked(&builtin::defaults(), Some(&dirs)).is_err());
    }

    #[gpui_kit::test]
    fn reload_switches_and_keeps_the_last_good_theme(cx: &mut TestAppContext) {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        cx.update(|cx| {
            gpui_kit::init(cx);
            ThemeEngine::install(Some(dirs.clone()), cx);
            assert_eq!(ThemeEngine::global(cx).active_id(), DEFAULT_ID);

            write(&dirs.config_file(), "theme = \"catppuccin-mocha\"");
            let notices = ThemeEngine::reload(Some(&dirs), cx);
            assert!(notices.iter().all(|n| n.contains("warning")), "{notices:?}");
            assert_eq!(ThemeEngine::global(cx).active_id(), "catppuccin-mocha");
            let mocha: gpui_kit::Hsla = gpui_kit::rgb(0x1e1e2e).into();
            assert_eq!(ThemeEngine::color(cx, Role::Background), mocha);
            assert_eq!(Theme::global(cx).colors.background, mocha);

            user_theme(&dirs, "catppuccin-mocha", "format_version = 1\nbroken = [");
            let notices = ThemeEngine::reload(Some(&dirs), cx);
            assert!(notices.iter().any(|n| n.contains("keeping")), "{notices:?}");
            assert_eq!(ThemeEngine::global(cx).active_id(), "catppuccin-mocha");
            assert_eq!(ThemeEngine::color(cx, Role::Background), mocha);
            let _ = cx.new(|_| ());
        });
    }

    #[gpui_kit::test]
    fn colors_override_the_theme_and_survive_a_theme_switch(cx: &mut TestAppContext) {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        write(
            &dirs.config_file(),
            "[colors]\nheading_5 = \"#c0392b\"\nheading = \"#102030\"\ncode_background = \"#11223340\"\nquote_bar = \"accent\"\nbogus = \"#fff000\"\n",
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            let notices = ThemeEngine::install(Some(dirs.clone()), cx);
            assert!(
                notices.iter().any(|n| n.contains("colors.bogus")),
                "{notices:?}"
            );
            let red: gpui_kit::Hsla = gpui_kit::rgb(0xc0392b).into();
            let navy: gpui_kit::Hsla = gpui_kit::rgb(0x102030).into();
            assert_eq!(ThemeEngine::color(cx, Role::Heading5), red);
            // Levels follow the overridden heading, except the one set itself.
            assert_eq!(ThemeEngine::color(cx, Role::Heading2), navy);
            assert_eq!(ThemeEngine::color(cx, Role::Heading), navy);
            // Alpha rides along; an alias reads the theme's color of that role.
            assert!(
                (ThemeEngine::color(cx, Role::CodeBackground).a - 0x40 as f32 / 255.0).abs() < 1e-3
            );
            assert_eq!(
                ThemeEngine::color(cx, Role::QuoteBar),
                ThemeEngine::color(cx, Role::Accent)
            );
            // Text itself is untouched.
            assert_ne!(ThemeEngine::color(cx, Role::Text), navy);

            ThemeEngine::set("catppuccin-mocha", cx).expect("switches");
            assert_eq!(ThemeEngine::color(cx, Role::Heading5), red);
            assert_eq!(ThemeEngine::color(cx, Role::Heading3), navy);

            // Removing the table and reloading goes back to the theme's colors.
            write(&dirs.config_file(), "theme = \"catppuccin-mocha\"\n");
            ThemeEngine::reload(Some(&dirs), cx);
            assert_eq!(
                ThemeEngine::color(cx, Role::Heading5),
                ThemeEngine::color(cx, Role::Text)
            );
            let _ = cx.new(|_| ());
        });
    }

    #[test]
    fn available_lists_built_ins_then_user_themes() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        let user = mini("").replace("Mini", "Zed Custom");
        user_theme(&dirs, "zed", &user);
        user_theme(&dirs, "broken", "format_version = 1\nbroken = [");
        user_theme(&dirs, "has space", &user);
        write(&dirs.theme_dir().join("notes.txt"), "x");
        let found = choices::available(&builtin::defaults(), Some(&dirs));
        let ids: Vec<_> = found.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids[0], DEFAULT_ID);
        assert_eq!(ids.len(), builtin::THEMES.len() + 1, "{ids:?}");
        assert_eq!(ids.last(), Some(&"zed"));
        assert_eq!(found.last().map(|c| c.name.as_str()), Some("Zed Custom"));
        let without = choices::available(&builtin::defaults(), None);
        assert_eq!(without.len(), builtin::THEMES.len());
    }

    #[test]
    fn the_vault_path_round_trips_and_keeps_other_keys() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        assert_eq!(
            crate::theme::configured_vault(Some(&dirs)),
            None,
            "no file yet"
        );

        write(&dirs.config_file(), "theme = \"a\"\n");
        assert_eq!(
            crate::theme::configured_vault(Some(&dirs)),
            None,
            "no vault key"
        );

        let vault = tmp.path().join("notes");
        choices::remember_vault(&dirs, &vault).expect("writes");
        assert_eq!(crate::theme::configured_vault(Some(&dirs)), Some(vault));
        let table: toml::Table = fs::read_to_string(dirs.config_file())
            .expect("read")
            .parse()
            .expect("toml");
        assert_eq!(table["theme"].as_str(), Some("a"));
    }

    #[test]
    fn an_empty_or_unreadable_vault_setting_counts_as_unset() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        write(&dirs.config_file(), "vault = \"  \"\n");
        assert_eq!(crate::theme::configured_vault(Some(&dirs)), None);
        write(&dirs.config_file(), "vault = [");
        assert_eq!(crate::theme::configured_vault(Some(&dirs)), None);
        assert_eq!(crate::theme::configured_vault(None), None);
    }

    #[test]
    fn remembering_the_vault_creates_a_missing_config_dir() {
        let tmp = tempdir().expect("tmp");
        let dirs = ConfigDirs::new(tmp.path().join("fresh").join("jade"));
        choices::remember_vault(&dirs, &tmp.path().join("notes")).expect("creates dirs");
        assert!(dirs.config_file().is_file());
    }

    #[test]
    fn remember_keeps_other_keys_and_never_clobbers_a_broken_file() {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        choices::remember(&dirs, "catppuccin-latte").expect("creates the file");
        let table: toml::Table = fs::read_to_string(dirs.config_file())
            .expect("read")
            .parse()
            .expect("toml");
        assert_eq!(table["theme"].as_str(), Some("catppuccin-latte"));

        write(&dirs.config_file(), "theme = \"a\"\ntext_size = 14\n");
        choices::remember(&dirs, "b").expect("rewrites");
        let table: toml::Table = fs::read_to_string(dirs.config_file())
            .expect("read")
            .parse()
            .expect("toml");
        assert_eq!(table["theme"].as_str(), Some("b"));
        assert_eq!(table["text_size"].as_integer(), Some(14));

        write(&dirs.config_file(), "theme = [");
        assert!(choices::remember(&dirs, "c").is_err());
        assert_eq!(
            fs::read_to_string(dirs.config_file()).expect("read"),
            "theme = ["
        );
    }

    #[gpui_kit::test]
    fn choose_switches_and_remembers(cx: &mut TestAppContext) {
        let tmp = tempdir().expect("tmp");
        let dirs = dirs_in(&tmp);
        cx.update(|cx| {
            gpui_kit::init(cx);
            ThemeEngine::install(Some(dirs.clone()), cx);
            ThemeEngine::choose("catppuccin-mocha", cx).expect("chosen");
            assert_eq!(ThemeEngine::global(cx).active_id(), "catppuccin-mocha");
            let startup = load_startup(&builtin::defaults(), Some(&dirs));
            assert_eq!(startup.id, "catppuccin-mocha");
            assert!(ThemeEngine::choose("nope", cx).is_err());
            assert_eq!(ThemeEngine::global(cx).active_id(), "catppuccin-mocha");
            let _ = cx.new(|_| ());
        });
    }

    use gpui_kit::component::{Theme, ThemeColor};

    /// Debug-prints a `ThemeColor` as `field -> value` lines.
    fn fields(colors: &ThemeColor) -> std::collections::BTreeMap<String, String> {
        let text = format!("{colors:#?}");
        let mut map = std::collections::BTreeMap::new();
        let mut current: Option<String> = None;
        for line in text.lines().skip(1) {
            if let Some(rest) = line.strip_prefix("    ").filter(|r| !r.starts_with(' ')) {
                if let Some((name, value)) = rest.split_once(": ") {
                    current = Some(name.to_owned());
                    map.insert(name.to_owned(), value.to_owned());
                    continue;
                }
            }
            if let Some(name) = &current {
                map.entry(name.clone())
                    .and_modify(|v| v.push_str(line.trim()));
            }
        }
        map
    }

    #[test]
    fn every_mapped_key_is_a_real_config_key() {
        let theme = super::load("mocha");
        let config = crate::theme::apply::build_config(&theme).expect("builds");
        let value = toml::Value::try_from(&config).expect("serializes");
        let colors = value
            .get("colors")
            .and_then(|c| c.as_table())
            .expect("colors table");
        for (key, _) in crate::theme::apply::MAPPING {
            assert!(
                colors.contains_key(*key),
                "gpui-component ignored the key {key:?}"
            );
        }
    }

    #[gpui_kit::test]
    fn applying_sets_widget_colors_and_leaves_no_stock_color(cx: &mut TestAppContext) {
        for (id, stock) in [
            ("catppuccin-mocha", ThemeColor::dark()),
            ("catppuccin-latte", ThemeColor::light()),
        ] {
            let tmp = tempdir().expect("tmp");
            let dirs = dirs_in(&tmp);
            write(&dirs.config_file(), format!("theme = \"{id}\""));
            cx.update(|cx| {
                gpui_kit::init(cx);
                ThemeEngine::install(Some(dirs), cx);
                let theme = Theme::global(cx);
                let engine = ThemeEngine::global(cx);
                assert_eq!(
                    theme.background,
                    engine.palette().color(Role::Background),
                    "{id}"
                );
                assert_eq!(theme.is_dark(), id.ends_with("mocha"));
                let (got, stock) = (fields(&theme.colors), fields(&stock));
                assert!(got.len() > 100, "parsed {} fields", got.len());
                let leaks: Vec<&String> = got
                    .iter()
                    .filter(|(k, v)| stock.get(*k) == Some(*v))
                    .map(|(k, _)| k)
                    .collect();
                assert!(leaks.is_empty(), "{id}: still stock: {leaks:?}");
            });
        }
    }
}

/// Colors come from the theme. A literal in UI code would ignore the theme
/// the user picked. Every file that paints UI is listed.
#[test]
fn ui_code_has_no_color_literals() {
    const THEMED: &[(&str, &str)] = &[
        ("app/mod.rs", include_str!("../app/mod.rs")),
        ("app/chrome.rs", include_str!("../app/chrome.rs")),
        ("app/dialogs.rs", include_str!("../app/dialogs.rs")),
        ("app/notes.rs", include_str!("../app/notes.rs")),
        ("app/search.rs", include_str!("../app/search.rs")),
        ("app/sidebar.rs", include_str!("../app/sidebar.rs")),
        ("app/tabs.rs", include_str!("../app/tabs.rs")),
        ("app/welcome.rs", include_str!("../app/welcome.rs")),
        ("element.rs", include_str!("../text_editor/element.rs")),
        (
            "element/input.rs",
            include_str!("../text_editor/element/input.rs"),
        ),
        (
            "element/paint.rs",
            include_str!("../text_editor/element/paint.rs"),
        ),
        (
            "element/region.rs",
            include_str!("../text_editor/element/region.rs"),
        ),
        (
            "element/render.rs",
            include_str!("../text_editor/element/render.rs"),
        ),
        (
            "element/table_paint.rs",
            include_str!("../text_editor/element/table_paint.rs"),
        ),
        (
            "element/widgets.rs",
            include_str!("../text_editor/element/widgets.rs"),
        ),
    ];
    const FORBIDDEN: &[&str] = &[
        "rgb(", "rgba(", "hsla(", "Hsla {", "white()", "black()", "0x",
    ];
    for (file, src) in THEMED {
        let ui = src.split("\n#[cfg(test)]\nmod ").next().unwrap_or_default();
        for (n, line) in ui.lines().enumerate() {
            let code = line
                .split("//")
                .next()
                .unwrap_or_default()
                .replace("transparent_black()", "")
                .replace("-> Hsla {", "");
            for bad in FORBIDDEN {
                assert!(
                    !code.contains(bad),
                    "{file}:{}: color literal `{bad}`: {line}",
                    n + 1
                );
            }
        }
    }
}

fn resolved(extra: &str) -> ResolvedTheme {
    ResolvedTheme::parse(&defaults(), &mini(extra)).expect("valid")
}

#[test]
fn a_style_role_follows_its_fallback_until_set() {
    let t = resolved("");
    let c = |role| t.color(role);
    assert_eq!(c(Role::Heading), c(Role::Text));
    for level in 1..=6 {
        assert_eq!(
            c(Role::heading_level(level)),
            c(Role::Text),
            "level {level}"
        );
    }
    assert_eq!(c(Role::QuoteBar), c(Role::BorderStrong));
    assert_eq!(c(Role::LinkUnderline), c(Role::Link));
    assert_eq!(c(Role::CodeBackground), c(Role::Surface));
}

#[test]
fn every_syntax_role_follows_text_until_set() {
    let t = resolved("");
    for role in Role::SYNTAX {
        assert!(role.is_syntax());
        assert_eq!(t.color(role), t.color(Role::Text), "{}", role.name());
    }
    assert!(!Role::Text.is_syntax() && !Role::Code.is_syntax());
    let t = resolved("\n[overrides]\nsyntax_keyword = \"#aa0000\"\ntext = \"#102030\"\n");
    assert_eq!(t.color(Role::SyntaxKeyword), rgb("aa0000"));
    assert_eq!(
        t.color(Role::SyntaxString),
        rgb("102030"),
        "the others follow text"
    );
}

#[test]
fn a_low_contrast_syntax_role_is_reported() {
    let t = resolved("\n[overrides]\nsyntax_comment = \"#fefefe\"\n");
    let bad: Vec<_> = t.warnings.iter().map(|w| w.foreground).collect();
    assert!(bad.contains(&Role::SyntaxComment), "{bad:?}");
}

#[test]
fn heading_colors_cascade_from_heading_to_a_level() {
    let t = resolved("\n[overrides]\nheading = \"#112233\"\nheading_5 = \"#445566\"\n");
    assert_eq!(t.color(Role::Heading), rgb("112233"));
    assert_eq!(t.color(Role::Heading1), rgb("112233"));
    assert_eq!(t.color(Role::Heading6), rgb("112233"));
    assert_eq!(t.color(Role::Heading5), rgb("445566"));
    // Text is untouched.
    assert_ne!(t.color(Role::Text), rgb("112233"));
}

#[test]
fn an_overridden_fallback_carries_to_the_roles_that_follow_it() {
    let t = resolved("\n[overrides]\nlink = \"#abcdef\"\ntext = \"#102030\"\n");
    assert_eq!(t.color(Role::LinkUnderline), rgb("abcdef"));
    assert_eq!(t.color(Role::Heading3), rgb("102030"));
}

#[test]
fn roles_toml_may_assign_a_role_that_has_a_fallback() {
    let with = ROLES.replace("[alpha]", "[roles_extra]\n[alpha]");
    assert!(
        Defaults::parse(&with).is_err(),
        "an unknown section is rejected"
    );
    let with = ROLES.replacen("[roles]\n", "[roles]\nheading = \"lavender\"\n", 1);
    let d = Defaults::parse(&with).expect("valid");
    let t = ResolvedTheme::parse(&d, &mini("")).expect("valid");
    assert_eq!(t.color(Role::Heading), t.color(Role::Accent));
    assert_eq!(t.color(Role::Heading2), t.color(Role::Accent));
}

#[test]
fn a_role_without_a_value_or_fallback_is_still_required() {
    let without = ROLES.replace("highlight = \"yellow\"", "");
    let errors = Defaults::parse(&without).expect_err("highlight is required");
    assert_eq!(
        errors,
        vec![ThemeError::MissingDefault { name: "highlight" }]
    );
}

#[test]
fn every_fallback_chain_ends_at_a_role_roles_toml_assigns() {
    let d = defaults();
    for &role in Role::ALL {
        let (mut current, mut steps) = (role, 0);
        while let Some(next) = current.fallback() {
            current = next;
            steps += 1;
            assert!(steps <= 3, "{} loops or is too deep", role.name());
        }
        assert!(
            d.assigns(current),
            "{} ends at {}, which roles.toml must assign",
            role.name(),
            current.name()
        );
    }
}

#[test]
fn a_low_contrast_heading_warns_once_not_once_per_level() {
    let t = resolved("\n[overrides]\nheading = \"#f0f0f0\"\nheading_3 = \"#f1f1f1\"\n");
    let headings: Vec<Role> = t
        .warnings
        .iter()
        .map(|w| w.foreground)
        .filter(|r| r.name().starts_with("heading"))
        .collect();
    assert_eq!(headings, vec![Role::Heading, Role::Heading3]);
}

#[test]
fn heading_level_maps_one_to_six_and_clamps() {
    assert_eq!(Role::heading_level(1), Role::Heading1);
    assert_eq!(Role::heading_level(6), Role::Heading6);
    assert_eq!(Role::heading_level(0), Role::Heading1);
    assert_eq!(Role::heading_level(9), Role::Heading6);
}

#[test]
fn overrides_apply_to_a_theme_and_leave_the_original() {
    use super::overrides::{ColorOverrides, new_warnings};
    let theme = resolved("");
    let (o, _) = ColorOverrides::parse("[colors]\ntext = \"#f8f8f8\"\nheading_2 = \"link\"\n");
    let applied = o.apply_to(&theme);
    assert_eq!(applied.theme.color(Role::Text), rgb("f8f8f8"));
    assert_eq!(
        applied.theme.color(Role::Heading),
        rgb("f8f8f8"),
        "follows text"
    );
    assert_eq!(applied.theme.color(Role::Heading2), theme.color(Role::Link));
    assert_eq!(
        theme.color(Role::Text),
        rgb("000000"),
        "the theme itself is unchanged"
    );
    // A near-white text on the white background is a new warning, once.
    let fresh = new_warnings(&theme, &applied.theme);
    assert!(
        fresh.iter().any(|w| w.contains("text on background")),
        "{fresh:?}"
    );
    assert!(new_warnings(&applied.theme, &applied.theme).is_empty());
    let (none, _) = ColorOverrides::parse("");
    assert_eq!(
        none.apply_to(&theme).theme.color(Role::Text),
        theme.color(Role::Text)
    );
}

#[test]
fn a_role_the_theme_colored_itself_does_not_follow_the_override() {
    use super::overrides::ColorOverrides;
    let theme = resolved("\n[overrides]\nheading_3 = \"#445566\"\n");
    let (o, _) = ColorOverrides::parse("[colors]\nheading = \"#102030\"\n");
    let t = o.apply_to(&theme).theme;
    assert_eq!(t.color(Role::Heading3), rgb("445566"));
    assert_eq!(t.color(Role::Heading4), rgb("102030"));
}

#[test]
fn every_shipped_theme_has_a_readable_and_varied_syntax_scheme() {
    for (name, _) in SHIPPED {
        let t = load(name);
        let bad: Vec<_> = t
            .warnings
            .iter()
            .filter(|w| w.foreground.is_syntax())
            .map(ToString::to_string)
            .collect();
        assert!(bad.is_empty(), "{name}: {bad:?}");
        let mut colors: Vec<_> = Role::SYNTAX.iter().map(|&r| t.color(r)).collect();
        colors.sort_by_key(|c| (c.r, c.g, c.b));
        colors.dedup();
        assert!(
            colors.len() >= 8,
            "{name}: only {} distinct colors",
            colors.len()
        );
        assert_ne!(t.color(Role::SyntaxKeyword), t.color(Role::Text), "{name}");
    }
}
