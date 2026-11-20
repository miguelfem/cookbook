# Computer-use attribution

The macOS event posting, accessibility lookup and OCR block-merging portions of
`computer_worker.py` are adapted from **typesafe-computer-use**, by **Aaron Levin**.

- Upstream: https://github.com/awlevin/typesafe-computer-use
- Revision: `7d120f1944d0156773d5089b81f8e46d108a6c8d`
- Original files: `typesafe_computer_use/macos.py` and `typesafe_computer_use/perception.py`
- License: MIT, Copyright (c) 2026 Aaron Levin. The full license is reproduced in
  `LICENSE.upstream` and in the adapted source.

APR adaptations use one frozen worker source, a foreground application allowlist,
window-only capture, minimal accessibility metadata, bounded actions and text
catalogs, stale-observation checks, bounded sessions, and durable APR effects.
No upstream credential handling, network client, automatic writer, full-display
logging, or autonomous execution loop is included. Jev requests use APR's
protected connection broker; caller decisions use APR's durable handoffs.

These third-party MIT terms apply to the identified adapted code and do not
select a license for the rest of APR.
