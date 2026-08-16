//! Batch segments into LLM calls. v0 translates at block (paragraph) granularity
//! with batched JSON in/out — this keeps alignment bulletproof (N in → N out, id
//! matched) which the research flags as the #1 sentence-level failure mode.
//!
//! A batch never crosses a chapter boundary and is bounded by both a unit count
//! and a soft token budget.

use crate::document::Chapter;

/// Indices (into `chapter.segments`) for the segments in one LLM call.
pub type Batch = Vec<usize>;

pub fn batches(chapter: &Chapter, max_units: usize, max_tokens: usize) -> Vec<Batch> {
    let mut out = Vec::new();
    let mut cur: Batch = Vec::new();
    let mut cur_tokens = 0usize;

    for (i, seg) in chapter.segments.iter().enumerate() {
        let t = seg.est_tokens();
        let would_overflow =
            !cur.is_empty() && (cur.len() >= max_units || cur_tokens + t > max_tokens);
        if would_overflow {
            out.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push(i);
        cur_tokens += t;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Segment;

    fn ch(n: usize) -> Chapter {
        Chapter {
            spine_index: 0,
            href: "x".into(),
            title: None,
            segments: (0..n)
                .map(|i| Segment::new(i, "word ".repeat(20), Default::default()))
                .collect(),
        }
    }

    #[test]
    fn respects_unit_cap() {
        let c = ch(25);
        let b = batches(&c, 10, 100_000);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].len(), 10);
        assert_eq!(b[2].len(), 5);
    }
}
