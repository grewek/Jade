mod context;
pub mod fence;
pub mod link_scan;
mod links;
mod note;
mod note_name;
mod rename;
mod resolve;
mod vault;

pub use context::{LinkContext, SourceContexts};
pub use links::{WikiLink, extract_wikilinks};
pub use note::Note;
pub use rename::Renamed;
pub use resolve::{LinkTarget, PathLinkError, percent_decode, split_fragment};
pub use vault::{SearchTarget, Vault, search_targets};
