//! Core of mdtui: markdown → IR → laid-out terminal lines, with a hit map for mouse interaction.
//!
//! Nothing in here touches the terminal, which keeps it easy to test.

pub mod highlight;
pub mod hitmap;
pub mod ir;
pub mod layout;
pub mod math;
pub mod parse;
pub mod search;
pub mod slug;
pub mod source;
pub mod theme;
mod wrap;

pub use hitmap::{Action, Hit, HitMap};
pub use ir::{Block, Document, Inline};
pub use layout::{layout, Layout, LayoutOptions, MediaInfo, MediaKind, MediaPlacement, MediaProvider};
pub use parse::{parse, ParseOptions};
pub use theme::Theme;
