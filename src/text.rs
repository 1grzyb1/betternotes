//! Text boxes: line layout from real font metrics, plus glyph outlines so
//! exporters can draw text as vector paths (no font embedding needed).

use std::sync::OnceLock;

use crate::model::P;

pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub const FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

/// Line height as a multiple of the font size.
pub const LINE_HEIGHT: f32 = 1.3;

fn face() -> &'static ttf_parser::Face<'static> {
    static FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| ttf_parser::Face::parse(FONT_REGULAR, 0).expect("bundled font is valid"))
}

#[derive(Clone, Debug, Default)]
pub struct TextLine {
    pub text: String,
    /// Offset of the line's top from the text box top.
    pub top: f32,
    pub width: f32,
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub lines: Vec<TextLine>,
    pub width: f32,
    pub height: f32,
    /// Distance from a line's top to its baseline.
    pub baseline: f32,
    /// Offset that centres the font's natural line box within `LINE_HEIGHT`.
    pub pad: f32,
}

fn advance(c: char, size: f32) -> f32 {
    let f = face();
    let scale = size / f.units_per_em() as f32;
    let gid = f.glyph_index(c).or_else(|| f.glyph_index('?'));
    gid.and_then(|g| f.glyph_hor_advance(g)).unwrap_or(0) as f32 * scale
}

pub fn measure(s: &str, size: f32) -> f32 {
    s.chars().map(|c| advance(c, size)).sum()
}

/// Greedy word wrap at `max_width`; words longer than a line are broken.
pub fn layout(text: &str, size: f32, max_width: f32) -> TextLayout {
    let f = face();
    let scale = size / f.units_per_em() as f32;
    let asc = f.ascender() as f32 * scale;
    let desc = f.descender() as f32 * scale;
    let line_h = size * LINE_HEIGHT;
    let pad = (line_h - (asc - desc)) * 0.5;
    let max_width = max_width.max(size);

    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut line_w = 0.0;
        for word in para.split_inclusive(' ') {
            let w = measure(word, size);
            let w_trim = measure(word.trim_end(), size);
            if line_w + w_trim <= max_width || line.is_empty() && w_trim <= max_width {
                line.push_str(word);
                line_w += w;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_w = 0.0;
            }
            if w_trim <= max_width {
                line.push_str(word);
                line_w = w;
            } else {
                // hard-break an overlong word
                for c in word.chars() {
                    let cw = advance(c, size);
                    if line_w + cw > max_width && !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                        line_w = 0.0;
                    }
                    line.push(c);
                    line_w += cw;
                }
            }
        }
        lines.push(line);
    }

    let lines: Vec<TextLine> = lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let l = l.trim_end().to_string();
            TextLine {
                width: measure(&l, size),
                top: i as f32 * line_h,
                text: l,
            }
        })
        .collect();
    TextLayout {
        width: lines.iter().map(|l| l.width).fold(0.0, f32::max),
        height: lines.len() as f32 * line_h,
        baseline: pad + asc,
        pad,
        lines,
    }
}

/// Vector path commands in page units.
#[derive(Clone, Copy, Debug)]
pub enum PathCmd {
    Move(P),
    Line(P),
    Quad(P, P),
    Cubic(P, P, P),
    Close,
}

struct Builder {
    out: Vec<PathCmd>,
    ox: f32,
    oy: f32,
    scale: f32,
}

impl Builder {
    fn p(&self, x: f32, y: f32) -> P {
        // font units are y-up
        P::new(self.ox + x * self.scale, self.oy - y * self.scale)
    }
}

impl ttf_parser::OutlineBuilder for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.out.push(PathCmd::Move(p));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.out.push(PathCmd::Line(p));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b) = (self.p(x1, y1), self.p(x, y));
        self.out.push(PathCmd::Quad(a, b));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b, c) = (self.p(x1, y1), self.p(x2, y2), self.p(x, y));
        self.out.push(PathCmd::Cubic(a, b, c));
    }
    fn close(&mut self) {
        self.out.push(PathCmd::Close);
    }
}

/// Glyph outlines for a laid-out text box whose top-left is at `pos`.
pub fn outlines(layout: &TextLayout, pos: P, size: f32) -> Vec<PathCmd> {
    let f = face();
    let scale = size / f.units_per_em() as f32;
    let mut b = Builder {
        out: Vec::new(),
        ox: 0.0,
        oy: 0.0,
        scale,
    };
    for line in &layout.lines {
        let mut x = pos.x;
        let baseline = pos.y + line.top + layout.baseline;
        for c in line.text.chars() {
            if let Some(gid) = f.glyph_index(c).or_else(|| f.glyph_index('?')) {
                b.ox = x;
                b.oy = baseline;
                f.outline_glyph(gid, &mut b);
                x += f.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;
            }
        }
    }
    b.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words() {
        let l = layout("The quick brown fox, zażółć gęślą jaźń", 12.0, 80.0);
        assert!(l.lines.len() > 1);
        assert!(l.lines.iter().all(|line| line.width <= 80.0 + 0.01));
        let l = layout("a\n\nb", 12.0, 200.0);
        assert_eq!(l.lines.len(), 3);
    }
}
