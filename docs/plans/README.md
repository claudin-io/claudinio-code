# Plan documents

One design document per non-trivial feature, written before the code: the
problem, the decisions and why they were made, the files touched, and often an
implementation log added afterwards.

**These are historical records, not documentation.** A plan describes what was
decided on the day it was written. The code has moved since; where the two
disagree, the code is right. Do not read a plan as a description of how the
system currently works — [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) and the
source are for that.

They are kept in the repository because the reasoning behind a decision is
usually harder to recover than the decision itself, and because reviewers of a
new change benefit from seeing how similar ones were argued.

Naming: `YYYY-MM-DD_slug.md`. Bug fixes and small changes do not need one.
