mod context;
pub mod fence;
pub mod link_scan;
mod links;
pub mod math_scan;
mod note;
mod note_name;
mod rename;
mod resolve;
#[cfg(test)]
mod test_support;
mod vault;

pub use context::{LinkContext, SourceContexts};
pub use links::{WikiLink, extract_wikilinks};
pub use note::Note;
pub use rename::Renamed;
pub use resolve::{Destination, image_file, percent_decode, split_fragment};
pub use vault::{SearchTarget, Vault, search_targets};
