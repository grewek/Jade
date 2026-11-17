//! Fixtures the vault-level tests share.

use crate::Vault;
use std::fs;

/// A vault in a temp directory holding `files` (relative path, text),
/// creating folders as needed. Keep the `TempDir` alive as long as the vault.
pub(crate) fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let full = dir.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }
    let vault = Vault::open(dir.path()).unwrap();
    (dir, vault)
}
