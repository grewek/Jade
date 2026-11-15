mod links;
mod note;
mod vault;

pub use links::{WikiLink, extract_wikilinks};
pub use note::Note;
pub use vault::{SearchTarget, Vault, search_targets};
