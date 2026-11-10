# Third-party source notices

The bounded Mermaid renderer in `crates/devtabs-mermaid/src/` and Unicode math recognition/layout in `src/preview/math.rs` and `src/preview/math/` derive from the local OpenAI Codex source snapshot, copied on 2026-11-10. The reference workspace identifies itself as version 0.0.0; its release/commit identity is not established.

Copyright 2025 OpenAI. Licensed under Apache License 2.0. The original license and notice are preserved in `licenses/codex/` and, for the standalone Mermaid crate, in its directory. The upstream NOTICE also credits Ratatui-derived code under MIT. File hashes and adaptation notes are recorded in `licenses/codex/provenance.json`.

The math prepass was adapted for pulldown-cmark 0.13, standard iterator chains, and the DevTabs width helper. Integration, themes, source mapping and the Markdown layout are maintained in DevTabs. Production builds do not require `refer/`.
