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

// This sink retains the precise node created while processing each anchor
// token. Keeping handles alive also prevents pointer reuse for detached nodes.
struct NodeTracingDom {
    dom: RcDom,
    recording: Cell<bool>,
    created: RefCell<Vec<Handle>>,
    retained: RefCell<Vec<Handle>>,
    offsets: RefCell<HashMap<*const markup5ever_rcdom::Node, usize>>,
}

impl Default for NodeTracingDom {
    fn default() -> Self {
        Self {
            dom: RcDom::default(),
            recording: Cell::new(false),
            created: RefCell::new(Vec::new()),
            retained: RefCell::new(Vec::new()),
            offsets: RefCell::new(HashMap::new()),
        }
    }
}

impl html5ever::tree_builder::TreeSink for NodeTracingDom {
    type Handle = Handle;
    type Output = Self;
    type ElemName<'a>
        = html5ever::ExpandedName<'a>
    where
        Self: 'a;
    fn finish(self) -> Self {
        self
    }
    fn parse_error(&self, msg: std::borrow::Cow<'static, str>) {
        self.dom.parse_error(msg);
    }
    fn get_document(&self) -> Handle {
        self.dom.get_document()
    }
    fn elem_name<'a>(&'a self, node: &'a Handle) -> Self::ElemName<'a> {
        self.dom.elem_name(node)
    }
    fn create_element(
        &self,
        name: html5ever::QualName,
        attrs: Vec<html5ever::Attribute>,
        flags: html5ever::tree_builder::ElementFlags,
    ) -> Handle {
        let anchor = name.local.as_ref() == "a";
        let handle = self.dom.create_element(name, attrs, flags);
        if self.recording.get() && anchor {
            self.created.borrow_mut().push(handle.clone());
        }
        handle
    }
    fn create_comment(&self, text: StrTendril) -> Handle {
        self.dom.create_comment(text)
    }
    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Handle {
        self.dom.create_pi(target, data)
    }
    fn append(&self, parent: &Handle, child: html5ever::tree_builder::NodeOrText<Handle>) {
        self.dom.append(parent, child);
    }
    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev: &Handle,
        child: html5ever::tree_builder::NodeOrText<Handle>,
    ) {
        self.dom.append_based_on_parent_node(element, prev, child);
    }
    fn append_doctype_to_document(&self, name: StrTendril, public: StrTendril, system: StrTendril) {
        self.dom.append_doctype_to_document(name, public, system);
    }
    fn get_template_contents(&self, target: &Handle) -> Handle {
        self.dom.get_template_contents(target)
    }
    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        self.dom.same_node(x, y)
    }
    fn set_quirks_mode(&self, mode: html5ever::tree_builder::QuirksMode) {
        self.dom.set_quirks_mode(mode);
    }
    fn append_before_sibling(
        &self,
        sibling: &Handle,
        child: html5ever::tree_builder::NodeOrText<Handle>,
    ) {
        self.dom.append_before_sibling(sibling, child);
    }
    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<html5ever::Attribute>) {
        self.dom.add_attrs_if_missing(target, attrs);
    }
    fn remove_from_parent(&self, target: &Handle) {
        self.dom.remove_from_parent(target);
    }
    fn reparent_children(&self, node: &Handle, parent: &Handle) {
        self.dom.reparent_children(node, parent);
    }
    fn is_mathml_annotation_xml_integration_point(&self, target: &Handle) -> bool {
        self.dom.is_mathml_annotation_xml_integration_point(target)
    }
    fn maybe_clone_an_option_into_selectedcontent(&self, option: &Handle) {
        self.dom.maybe_clone_an_option_into_selectedcontent(option);
    }
}

struct NodeTokenSink<'a> {
    builder: TreeBuilder<Handle, NodeTracingDom>,
    source: &'a str,
    end: Cell<usize>,
    barrier: Cell<usize>,
    eof: Cell<bool>,
}

impl TokenSink for NodeTokenSink<'_> {
    type Handle = Handle;
    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<Handle> {
        let attributes = match &token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag && tag.name.as_ref() == "a" => {
                Some(tag.attrs.clone())
            }
            _ => None,
        };
        let anchor = attributes.is_some();
        let position = if anchor && !self.eof.get() {
            let raw = &self.source[..self.end.get()];
            let first = crate::site::markup::next_open_tag(raw, "a", self.barrier.get());
            first.filter(|&start| crate::site::markup::next_open_tag(raw, "a", start + 1).is_none())
        } else {
            None
        };
        let is_boundary = matches!(
            token,
            Token::TagToken(_) | Token::CommentToken(_) | Token::DoctypeToken(_)
        );
        let sink = &self.builder.sink;
        sink.created.borrow_mut().clear();
        sink.recording.set(anchor);
        let result = self.builder.process_token(token, line);
        sink.recording.set(false);
        if let Some(offset) = position {
            let mut created = sink.created.borrow_mut();
            if created.len() == 1 {
                let node = created.pop().expect("one recorded node");
                let matches_token = matches!(&node.data, markup5ever_rcdom::NodeData::Element { attrs, .. } if attributes.as_ref().is_some_and(|expected| *attrs.borrow() == *expected));
                if matches_token {
                    sink.offsets
                        .borrow_mut()
                        .insert(std::rc::Rc::as_ptr(&node), offset);
                    sink.retained.borrow_mut().push(node);
                }
            }
        }
        if is_boundary {
            self.barrier.set(self.end.get());
        }
        result
    }
    fn end(&self) {
        self.builder.end();
    }
    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.builder
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

pub(super) fn trace_anchor_nodes(
    source: &str,
) -> Option<(RcDom, HashMap<*const markup5ever_rcdom::Node, usize>)> {
    if source.len() > crate::content_gate::MAX_CONTENT_INPUT_BYTES
        || source.bytes().filter(|b| *b == b'>').take(100_001).count() > 100_000
    {
        return None;
    }
    let builder = TreeBuilder::new(
        NodeTracingDom::default(),
        TreeBuilderOpts {
            scripting_enabled: false,
            ..Default::default()
        },
    );
    let tokenizer = Tokenizer::new(
        NodeTokenSink {
            builder,
            source,
            end: Cell::new(0),
            barrier: Cell::new(0),
            eof: Cell::new(false),
        },
        TokenizerOpts::default(),
    );
    let queue = BufferQueue::default();
    let mut start = 0;
    for end in source
        .match_indices('>')
        .map(|(i, _)| i + 1)
        .chain(std::iter::once(source.len()))
    {
        if end == start {
            continue;
        }
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
    tokenizer.sink.eof.set(true);
    tokenizer.end();
    let sink = tokenizer.sink.builder.sink;
    if !sink.dom.errors.borrow().is_empty() {
        return None;
    }
    Some((sink.dom, sink.offsets.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::trace_anchor_nodes;

    #[test]
    fn limits_and_parse_recovery_abstain() {
        assert!(trace_anchor_nodes(&">".repeat(100_001)).is_none());
        assert!(
            trace_anchor_nodes(&"x".repeat(crate::content_gate::MAX_CONTENT_INPUT_BYTES + 1))
                .is_none()
        );
        let html = "<!doctype html><html><head><title>x</title></head><body><main><p><a href='/one'><div>broken</div></a></main></body></html>";
        assert!(trace_anchor_nodes(html).is_none());
    }
}
