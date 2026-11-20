//! Jade's asset source: the Lucide icons the editor draws its controls with
//! that `gpui-kit`'s default bundle leaves out (they come from the same Lucide
//! set, license in `assets/icons/lucide/LICENSE`), in front of `gpui-kit`'s own.

use std::borrow::Cow;

use gpui_kit::AssetSource;
use gpui_kit::gpui::SharedString;

macro_rules! lucide {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/lucide/", $name, ".svg")).as_slice(),
        )),*]
    };
}

const EXTRA_ICONS: &[(&str, &[u8])] = lucide![
    "bug",
    "circle-question-mark",
    "clipboard-list",
    "flame",
    "grip-horizontal",
    "grip-vertical",
    "list",
    "pencil",
    "quote",
    "x",
    "zap",
];

pub struct JadeAssets;

impl AssetSource for JadeAssets {
    fn load(&self, path: &str) -> gpui_kit::gpui::Result<Option<Cow<'static, [u8]>>> {
        match EXTRA_ICONS.iter().find(|(name, _)| *name == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::gpui::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(
            EXTRA_ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
