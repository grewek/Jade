pub mod link_scan;
mod links;
mod note;
mod resolve;
mod vault;

pub use links::{WikiLink, extract_wikilinks};
pub use note::Note;
pub use resolve::{LinkTarget, PathLinkError, percent_decode, split_fragment};
pub use vault::{SearchTarget, Vault, search_targets};
