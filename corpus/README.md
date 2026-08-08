# Fidelity corpus

Test documents for `cargo xtask fidelity`. **These must be produced by real
Microsoft Word and Excel**, not by us and not by LibreOffice. The entire point is
to test against what actually lands in a user's inbox.

Drop files anywhere under `docx/` or `xlsx/`; the harness walks recursively and
picks up `.docx`, `.docm`, `.dotx`, `.xlsx`, `.xlsm`, `.xltx`.

## What to include

The harness is only as good as the awkwardness of its inputs. A folder of
one-paragraph documents proves nothing. Worth having:

**Word**
- tracked changes, and comments with replies
- nested tables, and tables that break across pages
- floating images with text wrap; inline images
- footnotes and endnotes
- multi-section documents with different page setups and orientations
- headers/footers that differ on first page and odd/even
- a table of contents, cross-references, and bookmarks
- RTL text (Arabic/Hebrew) and CJK text
- content controls / data-bound custom XML — the classic silent-loss case
- a document saved from a template (`.dotx`) with custom styles

**Excel**
- pivot tables, and slicers
- charts of several types
- conditional formatting and data validation
- array formulas and dynamic-array spills
- defined names, including sheet-scoped ones
- external workbook references
- merged cells, frozen panes, grouped rows/columns
- a sheet with 100k+ rows, for the performance checks later
- `.xlsm` with a VBA project — preserved verbatim, never executed

## Privacy

Anything in here is committed to the repo. Use documents you are comfortable
sharing, or scrub them first. Real-world structure is what matters, not real-world
content — a contract template with the names replaced is just as useful as the
signed original, and considerably less awkward.
