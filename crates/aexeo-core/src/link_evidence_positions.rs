//! Conservative source positions derived from the HTML5 tokenizer.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use html5ever::{
    TokenizerResult,
    buffer_queue::BufferQueue,
    tendril::StrTendril,
    tokenizer::{TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts},
    tree_builder::{TreeBuilder, TreeBuilderOpts},
};
use markup5ever_rcdom::{Handle, RcDom};

struct TracingSink<'a> {
    builder: TreeBuilder<Handle, RcDom>,
    source: &'a str,
    end: Cell<usize>,
    barrier: Cell<usize>,
    eof: Cell<bool>,
    anchors: RefCell<HashMap<String, Option<usize>>>,
}

impl TokenSink for TracingSink<'_> {
    type Handle = Handle;

    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<Handle> {
        if let Token::TagToken(tag) = &token {
            if !self.eof.get()
                && tag.kind == TagKind::StartTag
                && tag.name.as_ref() == "a"
                && let Some(href) = tag.attrs.iter().find(|a| a.name.local.as_ref() == "href")
            {
                let end = self.end.get();
                let raw = &self.source[..end];
                let first = crate::site::markup::next_open_tag(raw, "a", self.barrier.get());
                let unique = first.filter(|&start| {
                    crate::site::markup::next_open_tag(raw, "a", start + 1).is_none()
                });
                let mut anchors = self.anchors.borrow_mut();
                anchors
                    .entry(href.value.to_string())
                    .and_modify(|p| *p = None)
                    .or_insert(unique);
            }
            self.barrier.set(self.end.get());
        } else if matches!(token, Token::CommentToken(_) | Token::DoctypeToken(_)) {
            self.barrier.set(self.end.get());
        }
        self.builder.process_token(token, line)
    }

    fn end(&self) {
        self.builder.end();
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.builder
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

/// Returns positions only for unique source tokens in an error-free HTML parse.
pub(super) fn unique_anchor_offsets(source: &str) -> Option<HashMap<String, Option<usize>>> {
    // Bound both the parser input and the number of feed calls before building a DOM.
    if source.len() > crate::content_gate::MAX_CONTENT_INPUT_BYTES
        || source
            .bytes()
            .filter(|byte| *byte == b'>')
            .take(100_001)
            .count()
            > 100_000
    {
        return None;
    }
    let opts = TreeBuilderOpts {
        scripting_enabled: false,
        ..Default::default()
    };
    let builder = TreeBuilder::new(RcDom::default(), opts);
    let tokenizer = Tokenizer::new(
        TracingSink {
            builder,
            source,
            end: Cell::new(0),
            barrier: Cell::new(0),
            eof: Cell::new(false),
            anchors: RefCell::new(HashMap::new()),
        },
        TokenizerOpts::default(),
    );
    let queue = BufferQueue::default();
    // Feeding through each `>` bounds the possible source of an emitted tag.
    // The HTML5 tree builder still controls tokenizer states for scripts,
    // foreign markup and recovery. A second candidate in that interval abstains.
    let mut start = 0;
    for (i, _) in source.match_indices('>') {
        let end = i + 1;
        tokenizer.sink.end.set(end);
        queue.push_back(StrTendril::from_slice(&source[start..end]));
        while !queue.is_empty() {
            if matches!(tokenizer.feed(&queue), TokenizerResult::Done) {
                break;
            }
        }
        if !queue.is_empty() {
            return None;
        }
        start = end;
    }
    if start < source.len() {
        tokenizer.sink.end.set(source.len());
        queue.push_back(StrTendril::from_slice(&source[start..]));
        while !queue.is_empty() {
            if matches!(tokenizer.feed(&queue), TokenizerResult::Done) {
                break;
            }
        }
        if !queue.is_empty() {
            return None;
        }
    }
    tokenizer.sink.eof.set(true);
    tokenizer.end();
    if !tokenizer.sink.builder.sink.errors.borrow().is_empty() {
        return None;
    }
    Some(tokenizer.sink.anchors.into_inner())
}

#[cfg(test)]
mod tests {
    use super::unique_anchor_offsets;

    #[test]
    fn tokenized_positions_ignore_scripts_comments_and_quoted_angles() {
        let html = "<!doctype html><html><head><title>x</title><script>const s = '<a href=\"/fake\">';</script></head><body><!-- <a href='/fake'> --><main><svg><path d='M0 0'></path></svg><p>é <a title='1 > 0' href='/real?x=1&amp;y=2'>read</a></p></main></body></html>";
        let map = unique_anchor_offsets(html).expect("error-free parse");
        assert_eq!(map.get("/real?x=1&y=2"), Some(&html.find("<a title")));
        assert!(!map.contains_key("/fake"));
    }

    #[test]
    fn duplicate_destinations_and_ambiguous_angle_brackets_abstain() {
        let html = "<!doctype html><html><head><title>x</title></head><body><main><a href='/same'>one</a><a href='/same'>two</a><a title=\"<a href='/fake'\" href='/unique'>three</a></main></body></html>";
        let map = unique_anchor_offsets(html).expect("error-free parse");
        assert_eq!(map.get("/same"), Some(&None));
        assert_eq!(map.get("/unique"), Some(&None));
    }

    #[test]
    fn input_and_feed_limits_abstain_before_parsing() {
        assert!(unique_anchor_offsets(&">".repeat(100_001)).is_none());
        assert!(
            unique_anchor_offsets(&"x".repeat(crate::content_gate::MAX_CONTENT_INPUT_BYTES + 1))
                .is_none()
        );
    }

    #[test]
    fn parse_recovery_never_provides_positions() {
        let html = "<!doctype html><html><head><title>x</title></head><body><main><p><a href='/one'><div>broken</div></a></main></body></html>";
        assert!(unique_anchor_offsets(html).is_none());
    }
}
