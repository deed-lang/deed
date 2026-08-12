//! Byte ranges into a single source file.
//!
//! Spans are file relative. The owning [`FileId`](crate::FileId) lives on the
//! diagnostic, not on every span, because spans are copied constantly and
//! diagnostics are not.

/// A half open byte range `[start, end)` into one source file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// Creates a span covering `[start, end)`.
    ///
    /// # Panics
    ///
    /// Panics if `end < start`. A reversed span is always a bug in the caller
    /// rather than something to recover from.
    pub fn new(start: u32, end: u32) -> Self {
        assert!(end >= start, "span end {end} precedes start {start}");
        Self { start, end }
    }

    /// An empty span at `offset`, used to point between two characters.
    pub fn at(offset: u32) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn contains(self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }

    pub fn contains_span(self, other: Span) -> bool {
        other.start >= self.start && other.end <= self.end
    }

    pub fn as_range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

#[cfg(test)]
mod tests {
    use super::Span;

    #[test]
    fn union_covers_both() {
        let a = Span::new(2, 5);
        let b = Span::new(10, 12);
        assert_eq!(a.to(b), Span::new(2, 12));
        assert_eq!(b.to(a), Span::new(2, 12));
    }

    #[test]
    fn empty_span_contains_nothing() {
        let s = Span::at(4);
        assert!(s.is_empty());
        assert!(!s.contains(4));
    }

    #[test]
    fn a_span_contains_itself_and_spans_strictly_inside_it() {
        let outer = Span::new(2, 8);
        assert!(outer.contains_span(outer));
        assert!(outer.contains_span(Span::new(3, 7)));
    }

    #[test]
    fn a_span_does_not_contain_one_reaching_past_either_edge() {
        let outer = Span::new(2, 8);
        assert!(!outer.contains_span(Span::new(1, 7)));
        assert!(!outer.contains_span(Span::new(3, 9)));
    }

    #[test]
    fn end_is_exclusive() {
        let s = Span::new(1, 3);
        assert!(s.contains(1));
        assert!(s.contains(2));
        assert!(!s.contains(3));
    }
}
