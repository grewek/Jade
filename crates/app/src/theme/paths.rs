//! Where Jade's config lives. Resolution is a pure function of its inputs
//! so tests never touch the real config directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Overrides the config directory in every build (tests, the debug harness).
pub const CONFIG_DIR_ENV: &str = "JADE_CONFIG_DIR";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDirs {
    base: PathBuf,
}

impl ConfigDirs {
    pub fn new(base: PathBuf) -> ConfigDirs {
        ConfigDirs { base }
    }

    /// `override_dir` wins. Otherwise debug builds use `target/jade-dev`, so
    /// development never writes into the user's real config; release builds
    /// use `<os config dir>/jade`. `None` when the OS has no config dir.
    pub fn resolve(
        override_dir: Option<OsString>,
        os_config_dir: Option<PathBuf>,
        dev_build: bool,
    ) -> Option<ConfigDirs> {
        if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
            return Some(ConfigDirs::new(PathBuf::from(dir)));
        }
        if dev_build {
            return Some(ConfigDirs::new(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/jade-dev"),
            ));
        }
        os_config_dir.map(|dir| ConfigDirs::new(dir.join("jade")))
    }

    /// Reads the process environment; only `run()` calls this.
    pub fn from_environment() -> Option<ConfigDirs> {
        ConfigDirs::resolve(
            std::env::var_os(CONFIG_DIR_ENV),
            dirs::config_dir(),
            cfg!(debug_assertions),
        )
    }

    pub fn theme_dir(&self) -> PathBuf {
        self.base.join("theme")
    }

    pub fn config_file(&self) -> PathBuf {
        self.base.join("config.toml")
    }
}
