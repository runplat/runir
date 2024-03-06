use std::collections::BTreeMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;

use crate::define_intern_table;
use crate::push_tag;

use crate::prelude::*;

// Intern table for symbol values
define_intern_table!(SYMBOL: String);

// Intern table for input values
define_intern_table!(INPUT: String);

// Intern table for tag values
define_intern_table!(TAG: String);

// Intern table for path values
define_intern_table!(PATH: String);

// Intern table for node index values
define_intern_table!(NODE_IDX: usize);

// Intern table for block idx values
define_intern_table!(BLOCK_IDX: usize);

// Intern table for source
define_intern_table!(SOURCE: String);

// Intern table for doc headers
define_intern_table!(DOC_HEADERS: Vec<String>);

// Intern table for node level annotations
define_intern_table!(ANNOTATIONS: BTreeMap<String, String>);

// Intern table for node level source spans
define_intern_table!(SOURCE_SPAN: SourceSpan);

// Intern table for node level source relative paths
define_intern_table!(SOURCE_RELATIVE: PathBuf);

/// Type-alias for start-and-end positions from the node's source,
///
pub type SourceSpan = Range<usize>;

type NodeAnnotations = Tag<BTreeMap<String, String>, Arc<BTreeMap<String, String>>>;

/// Node level is a dynamic level of representation,
///
/// Node level asserts and records the input paramters for some resource as well as ordinality.
///
#[derive(Clone)]
pub struct NodeLevel {
    /// Symbol representing this node,
    ///
    symbol: Option<Tag<String, Arc<String>>>,
    /// Runmd expression representing this resource,
    ///
    input: Option<Tag<String, Arc<String>>>,
    /// Tag value assigned to this resource,
    ///
    tag: Option<Tag<String, Arc<String>>>,
    /// Path value assigned to this resource,
    ///
    path: Option<Tag<String, Arc<String>>>,
    /// Node idx,
    ///
    idx: Option<Tag<usize, Arc<usize>>>,
    /// Block idx,
    ///
    bidx: Option<Tag<usize, Arc<usize>>>,
    /// Node source,
    ///
    source: Option<Tag<String, Arc<String>>>,
    /// Node doc headers,
    ///
    doc_headers: Option<Tag<Vec<String>, Arc<Vec<String>>>>,
    /// Node annotations,
    ///
    annotations: Option<NodeAnnotations>,
    /// Position in source this node was parsed from,
    ///
    span: Option<Tag<SourceSpan, Arc<SourceSpan>>>,
    /// Relative path name of the source for this node,
    ///
    relative: Option<Tag<PathBuf, Arc<PathBuf>>>,
}

impl Default for NodeLevel {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeLevel {
    /// Returns a new empty node level,
    ///
    pub fn new() -> Self {
        Self {
            symbol: None,
            input: None,
            tag: None,
            path: None,
            idx: None,
            bidx: None,
            source: None,
            doc_headers: None,
            annotations: None,
            span: None,
            relative: None,
        }
    }

    /// Creates a new input level representation,
    ///
    #[allow(unused)] // Used in test
    #[allow(clippy::too_many_arguments)] // Used in test
    pub(crate) fn new_with(
        symbol: Option<impl Into<String>>,
        input: Option<impl Into<String>>,
        tag: Option<impl Into<String>>,
        path: Option<impl Into<String>>,
        idx: Option<usize>,
        block: Option<usize>,
        source: Option<impl Into<String>>,
        doc_headers: Option<Vec<impl Into<String>>>,
        annotations: Option<BTreeMap<String, String>>,
    ) -> Self {
        let mut node = Self::new();

        if let Some(symbol) = symbol {
            node.set_symbol(symbol);
        }
        if let Some(input) = input {
            node.set_input(input);
        }
        if let Some(tag) = tag {
            node.set_tag(tag)
        }
        if let Some(path) = path {
            node.set_path(path);
        }
        if let Some(idx) = idx {
            node.set_idx(idx);
        }
        if let Some(idx) = block {
            node.set_block(idx);
        }
        if let Some(source) = source {
            node.set_source(source);
        }
        if let Some(doc_headers) = doc_headers {
            node.set_doc_headers(doc_headers);
        }
        if let Some(annotations) = annotations {
            node.set_annotations(annotations);
        }

        node
    }

    /// Returns the node level w/ symbol tag set,
    ///
    #[inline]
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.set_symbol(symbol);
        self
    }

    /// Returns the node level w/ input tag set,
    ///
    #[inline]
    pub fn with_input(mut self, input: impl Into<String>) -> Self {
        self.set_input(input);
        self
    }

    /// Returns the node level w/ tag tag set,
    ///
    #[inline]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.set_tag(tag);
        self
    }

    /// Returns the node level w/ path tag set,
    ///  
    #[inline]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.set_path(path);
        self
    }

    /// Returns the node level w/ idx tag set,
    ///
    #[inline]
    pub fn with_idx(mut self, idx: usize) -> Self {
        self.set_idx(idx);
        self
    }

    /// Returns the node level w/ idx tag set,
    ///
    #[inline]
    pub fn with_block(mut self, idx: usize) -> Self {
        self.set_block(idx);
        self
    }

    /// Returns the node level w/ source set,
    ///
    #[inline]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.set_source(source);
        self
    }

    /// Returns the node level w/ doc headers set,
    ///
    #[inline]
    pub fn with_doc_headers(mut self, doc_headers: Vec<impl Into<String>>) -> Self {
        self.set_doc_headers(doc_headers);
        self
    }

    /// Returns the node level w/ annotations set,
    ///
    #[inline]
    pub fn with_annotations(mut self, annotations: BTreeMap<String, String>) -> Self {
        self.set_annotations(annotations);
        self
    }

    /// Returns the node level w/ source span set,
    ///
    #[inline]
    pub fn with_source_span(mut self, span: SourceSpan) -> Self {
        self.set_source_span(span);
        self
    }

    /// Returns the node level w/ relative source path set,
    ///
    #[inline]
    pub fn with_source_relative(mut self, relative: PathBuf) -> Self {
        self.set_source_relative(relative);
        self
    }

    /// Sets the symbol tag for the node level,
    ///
    #[inline]
    pub fn set_symbol(&mut self, symbol: impl Into<String>) {
        self.symbol = Some(Tag::new(&SYMBOL, Arc::new(symbol.into())));
    }

    /// Returns the node level w/ tag tag set,
    ///
    #[inline]
    pub fn set_input(&mut self, input: impl Into<String>) {
        self.input = Some(Tag::new(&INPUT, Arc::new(input.into())));
    }

    /// Returns the node level w/ tag tag set,
    ///
    #[inline]
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.tag = Some(Tag::new(&TAG, Arc::new(tag.into())));
    }

    /// Sets the path tag for the node level,
    ///
    #[inline]
    pub fn set_path(&mut self, path: impl Into<String>) {
        self.path = Some(Tag::new(&PATH, Arc::new(path.into())));
    }

    /// Returns the node level w/ idx tag set,
    ///
    #[inline]
    pub fn set_idx(&mut self, idx: usize) {
        self.idx = Some(Tag::new(&NODE_IDX, Arc::new(idx)));
    }

    /// Returns the node level w/ block idx tag set,
    ///
    #[inline]
    pub fn set_block(&mut self, idx: usize) {
        self.bidx = Some(Tag::new(&BLOCK_IDX, Arc::new(idx)));
    }

    /// Sets the source tag for the node level,
    ///
    #[inline]
    pub fn set_source(&mut self, source: impl Into<String>) {
        self.source = Some(Tag::new(&SOURCE, Arc::new(source.into())));
    }

    /// Sets the doc headers tag for the node level,
    ///
    #[inline]
    pub fn set_doc_headers(&mut self, mut headers: Vec<impl Into<String>>) {
        self.doc_headers = Some(Tag::new(
            &DOC_HEADERS,
            Arc::new(headers.drain(..).map(|s| s.into()).collect()),
        ))
    }

    /// Sets the node level annotations,
    ///
    #[inline]
    pub fn set_annotations(&mut self, annotations: BTreeMap<String, String>) {
        self.annotations = Some(Tag::new(&ANNOTATIONS, Arc::new(annotations)));
    }

    /// Sets the node level source span,
    ///
    #[inline]
    pub fn set_source_span(&mut self, span: SourceSpan) {
        self.span = Some(Tag::new(&SOURCE_SPAN, Arc::new(span)));
    }

    /// Sets the node level source relative path,
    ///
    #[inline]
    pub fn set_source_relative(&mut self, relative: PathBuf) {
        self.relative = Some(Tag::new(&SOURCE_RELATIVE, Arc::new(relative)));
    }
}

impl Level for NodeLevel {
    fn configure(&self, interner: &mut impl InternerFactory) -> InternResult {
        if let Some(symbol) = self.symbol.as_ref() {
            push_tag!(dyn interner, symbol);
        }

        if let Some(input) = self.input.as_ref() {
            push_tag!(dyn interner, input);
        }

        if let Some(tag) = self.tag.as_ref() {
            push_tag!(dyn interner, tag);
        }

        if let Some(path) = self.path.as_ref() {
            push_tag!(dyn interner, path);
        }

        if let Some(idx) = self.idx.as_ref() {
            push_tag!(dyn interner, idx);
        }

        if let Some(docs) = self.doc_headers.as_ref() {
            push_tag!(dyn interner, docs);
        }

        if let Some(source) = self.source.as_ref() {
            push_tag!(dyn interner, source);
        }

        if let Some(annotations) = self.annotations.as_ref() {
            push_tag!(dyn interner, annotations);
        }

        if let Some(source_span) = self.span.as_ref() {
            push_tag!(dyn interner, source_span);
        }

        if let Some(source_relative) = self.relative.as_ref() {
            push_tag!(dyn interner, source_relative);
        }

        interner.set_level_flags(LevelFlags::LEVEL_2);

        interner.interner()
    }

    type Mount = (
        // Symbol
        Option<Arc<String>>,
        // Input
        Option<Arc<String>>,
        // Tag
        Option<Arc<String>>,
        // Path
        Option<Arc<String>>,
        // Doc headers
        Option<Arc<Vec<String>>>,
        // Annotations
        Option<Arc<BTreeMap<String, String>>>,
    );

    #[inline]
    fn mount(&self) -> Self::Mount {
        (
            self.symbol.as_ref().map(|i| i.create_value.clone()),
            self.input.as_ref().map(|i| i.create_value.clone()),
            self.tag.as_ref().map(|t| t.create_value.clone()),
            self.path.as_ref().map(|p| p.create_value.clone()),
            self.doc_headers.as_ref().map(|t| t.create_value.clone()),
            self.annotations.as_ref().map(|a| a.create_value.clone()),
        )
    }
}

/// Wrapper struct with access to node tags,
///
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Serialize, Deserialize)]
pub struct NodeRepr(pub(crate) InternHandle);

impl NodeRepr {
    /// Returns the node symbol,
    ///
    pub fn symbol(&self) -> Option<Arc<String>> {
        self.0.symbol()
    }

    /// Returns node input,
    ///
    #[inline]
    pub fn input(&self) -> Option<Arc<String>> {
        self.0.input()
    }

    /// Returns node path,
    ///
    #[inline]
    pub fn path(&self) -> Option<Arc<String>> {
        self.0.path()
    }

    /// Returns node tag,
    ///
    #[inline]
    pub fn tag(&self) -> Option<Arc<String>> {
        self.0.tag()
    }

    /// Returns the node idx,
    ///
    #[inline]
    pub fn idx(&self) -> Option<usize> {
        self.0.node_idx()
    }

    /// Returns the node source,
    ///
    #[inline]
    pub fn source(&self) -> Option<Arc<String>> {
        self.0.node_source()
    }

    /// Returns node doc_headers,
    ///
    #[inline]
    pub fn doc_headers(&self) -> Option<Arc<Vec<String>>> {
        self.0.doc_headers()
    }

    /// Returns node annotations,
    ///
    #[inline]
    pub fn annotations(&self) -> Option<Arc<BTreeMap<String, String>>> {
        self.0.annotations()
    }

    /// Returns node source span,
    ///
    #[inline]
    pub fn span(&self) -> Option<Arc<SourceSpan>> {
        self.0.source_span()
    }

    /// Returns node source relative path,
    ///
    #[inline]
    pub fn relative(&self) -> Option<Arc<PathBuf>> {
        self.0.source_relative()
    }
}
