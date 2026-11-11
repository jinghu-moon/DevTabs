//! Rectangular clipping of encoded runs. No pixel decode or palette quantization.
use super::*;
use std::fmt::Write;

impl Sixel {
    /// Crop in character cells, preserving the original palette and exact pixel alignment.
    /// Intended for cached composition; generated Sixel always fits `crop` including
    /// font heights that are not multiples of the six-pixel Sixel band height.
    pub fn crop_cells(&self, crop: Rect, cell: Size) -> Option<Self> {
        if self.is_tmux || crop.is_empty() || cell.width == 0 || cell.height == 0 {
            return None;
        }
        let start = self.data.find("\x1bP")?;
        let data = &self.data[start..];
        let q = data.find('q')?;
        let end = data.find("\x1b\\")?;
        let bytes = data.as_bytes();
        let mut i = q + 1;
        let mut palette = String::new();
        // The encoder emits raster attributes, then palette definitions, then runs.
        while i < end {
            let begin = i;
            match bytes[i] {
                b'"' => {
                    i += 1;
                    while i < end && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                        i += 1;
                    }
                }
                b'#' => {
                    i += 1;
                    while i < end && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i >= end || bytes[i] != b';' {
                        i = begin;
                        break;
                    }
                    while i < end && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                        i += 1;
                    }
                    palette.push_str(&data[begin..i]);
                }
                _ => break,
            }
        }
        let x = u32::from(crop.x) * u32::from(cell.width);
        let y = u32::from(crop.y) * u32::from(cell.height);
        let width = u32::from(crop.width) * u32::from(cell.width);
        let height = u32::from(crop.height) * u32::from(cell.height);
        // Record the inherited palette selector, which can span band boundaries.
        let mut color = "#0";
        let bands: Vec<_> = data[i..end]
            .split('-')
            .map(|band| {
                let initial = color;
                let b = band.as_bytes();
                let mut p = 0;
                while p < b.len() {
                    if b[p] == b'#' {
                        let first = p;
                        p += 1;
                        while p < b.len() && (b[p].is_ascii_digit() || b[p] == b';') {
                            p += 1;
                        }
                        color = &band[first..p];
                    } else {
                        p += 1;
                    }
                }
                (band, initial)
            })
            .collect();
        let mut out = String::new();
        clear_area(&mut out, "\x1b", crop.width, crop.height);
        // Transparent zero bits are essential: the final six-pixel band may
        // extend beyond the last cell. Its masked padding must never erase a panel.
        write!(out, "\x1bP0;1;0q\"1;1;{width};{height}{palette}").ok()?;
        for dy in (0..height).step_by(6) {
            let first = (y + dy) / 6;
            let last = (y + (dy + 5).min(height - 1)) / 6;
            for band in first..=last {
                if let Some((runs, initial)) = bands.get(band as usize) {
                    out.push_str(initial);
                    let shift = i64::from(band * 6) - i64::from(y + dy);
                    crop_runs(&mut out, runs, x, width, shift, (height - dy).min(6));
                    out.push('$');
                }
            }
            // Do not advance after the last band: some decoders extend raster height.
            if dy + 6 < height {
                out.push('-');
            }
        }
        out.push_str("\x1b\\");
        Some(Self {
            data: out,
            size: crop.as_size(),
            is_tmux: false,
        })
    }
}

fn crop_runs(out: &mut String, runs: &str, left: u32, width: u32, shift: i64, rows: u32) {
    let bytes = runs.as_bytes();
    let mut i = 0;
    let mut x = 0u32;
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                let first = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b';') {
                    i += 1;
                }
                out.push_str(&runs[first..i]);
            }
            b'$' => {
                out.push('$');
                x = 0;
                i += 1;
            }
            b'!' | b'?'..=b'~' => {
                let mut count = 1u32;
                if bytes[i] == b'!' {
                    i += 1;
                    count = 0;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        count = count
                            .saturating_mul(10)
                            .saturating_add(u32::from(bytes[i] - b'0'));
                        i += 1;
                    }
                }
                if i >= bytes.len() || !(b'?'..=b'~').contains(&bytes[i]) {
                    break;
                }
                let bits = u32::from(bytes[i] - b'?');
                let bits = if shift < 0 {
                    bits >> (-shift as u32)
                } else {
                    bits << shift as u32
                };
                let bits = bits & ((1 << rows) - 1);
                let end = x.saturating_add(count);
                let visible = end.min(left + width).saturating_sub(x.max(left));
                if visible > 0 {
                    let ch = char::from(b'?' + bits as u8);
                    if visible > 3 {
                        let _ = write!(out, "!{visible}{ch}");
                    } else {
                        for _ in 0..visible {
                            out.push(ch);
                        }
                    }
                }
                x = end;
                i += 1;
            }
            _ => i += 1,
        }
    }
}
