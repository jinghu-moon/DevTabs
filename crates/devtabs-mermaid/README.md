# DevTabs Mermaid renderer

Vendored from `refer/codex-main/codex-rs/mermaid` on 2026-11-10. Source is unchanged. Original workspace version is 0.0.0; no release/commit identity was established. Production uses this directory, independent of refer/. See LICENSE, NOTICE and ../../THIRD-PARTY-NOTICES.md.

Bounded Mermaid subsets only; unsupported, overwide and oversized inputs return errors. Rendering emits semantic spans, not terminal escape sequences. Upstream tests and expected snapshot contents are retained; snapshot filenames use the local crate prefix. The workspace suite runs these tests as well as DevTabs preview integration tests.
