# Acknowledgments

Translatus did not invent its category. This page names the projects and
research whose ideas shaped it, and is specific about what we took — and what
we did not.

A note on method: everything below was **behavioral study, not code reuse**.
We ran or read about these tools as users and as researchers, wrote down what
worked, and implemented our own engine from scratch in Rust. No source code
from any of them was copied into Translatus. For the GPLv3 project this was a
deliberate clean-room boundary: mechanisms and algorithms are not
copyrightable; code is, and none crossed over.

## Category pioneers

### bilingual_book_maker (MIT)

<https://github.com/yihong0618/bilingual_book_maker>

The project that proved the category: point a CLI at an EPUB with your own
API key, get a bilingual book back. Its "translation as an independent
sibling element" output convention is the approach the whole category
(including us) converged on. Where Translatus differs: a byte-faithful XHTML
mini-DOM that only replaces text nodes (instead of re-serializing the
document), a content-addressed SQLite cache that survives renames and resumes
without re-billing, and hard per-segment alignment validation.

### Ebook-Translator Calibre plugin (GPLv3)

<https://github.com/bookfere/Ebook-Translator-Calibre-Plugin>

The most feature-complete tool in the category, and the one we studied most
carefully — always at the behavior level, never at the code level (it is
GPLv3; our clean-room verification record is kept internally). Ideas it
proved that we reimplemented in our own way: cache-only re-rendering
(decoupling "translate" from "render" so layout changes cost zero API calls),
position modes for where the translation lands, and disciplined
failure-circuit-breaking around flaky providers. Where Translatus differs:
strict 1:1 JSON-aligned batching instead of physically merged paragraphs, a
paired-token placeholder protocol with multiset validation, and an engine
that is a standalone library/CLI rather than a reader plugin.

## Expert-mode lineage

### translation-agent (Andrew Ng)

<https://github.com/andrewyng/translation-agent>

The clearest public demonstration that a translate → reflect → improve loop
beats single-shot translation, and that the reflection step must see the
source text, not just the draft. Translatus's expert mode adopts exactly that
discipline: our reflection pass is source-aware, so it can catch omissions
and mistranslations rather than merely polishing fluency.

### DelTA (ICLR 2025)

Wang et al., *DelTA: An Online Document-Level Translation Agent Based on
Multi-Level Memory* — <https://arxiv.org/abs/2410.08143>

The research argument that document-level quality comes from structured
memory (proper-noun records, bilingual summaries, long/short-term memory),
not from stuffing a bigger context window. That framing is the blueprint for
Translatus's whole-book passes: glossary pre-scan with hard enforcement,
rolling summaries, and a final consistency review.

---

If you believe your project should be listed here, or that a description
above is inaccurate, please open an issue — we are happy to correct the
record.
