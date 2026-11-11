# DevTabs patch to ratatui-image 11.1.0

Upstream: https://crates.io/crates/ratatui-image/11.1.0 (MIT; LICENSE retained).

The source retains upstream protocol rendering and image widgets. Local changes:

- `Sixel::with_options` passes one real `icy_sixel::EncodeOptions` through the existing encoder and clearing sequence. `Sixel::new` keeps upstream defaults. No second quantization.
- `SlicedProtocol::from_sixel` accepts that same already encoded Sixel; no second encoding.
- Vertical Sixel slicing subtracts skipped bands from the number of bands to take. The upstream count could exceed the remaining viewport after a top clip. Conservative six-pixel band rounding is retained; up to five edge pixels can be omitted. Integration tests decode cropped output at a 17-pixel cell height and enforce viewport bounds.
- `Halfblocks::retained_bytes` and `SlicedProtocol::retained_bytes` expose allocation capacities including the Sixel slice index, so the frontend can charge held cache memory.
- The package manifest omits the unused CLI/examples/bench targets. Optional Chafa backends remain disabled in the root manifest.

Production uses icy_sixel 0.5.1. The 0.7.0 alias is a development-only comparison candidate. Remove this patch once upstream public APIs support the profile/slicing/memory entry points and the clipping regression is fixed; rerun `image_preview`, `markdown_images` and real Windows Terminal acceptance before removing it.
