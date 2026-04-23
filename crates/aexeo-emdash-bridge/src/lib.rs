#![forbid(unsafe_code)]

pub mod document;
pub mod page;
pub mod portable_text;
pub mod render;
pub mod site;

pub use document::{EmdashDocument, HreflangAlternate};
pub use page::build_page_from_document;
pub use portable_text::{
    BlockStyle, ListItem, MarkDef, PortableTextBlock, PortableTextChild, PortableTextSpan,
};
pub use render::render_html;
pub use site::build_site_from_documents;
