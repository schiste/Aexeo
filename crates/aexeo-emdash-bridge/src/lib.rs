#![forbid(unsafe_code)]

pub mod portable_text;

pub use portable_text::{
    BlockStyle, ListItem, MarkDef, PortableTextBlock, PortableTextChild, PortableTextSpan,
};
