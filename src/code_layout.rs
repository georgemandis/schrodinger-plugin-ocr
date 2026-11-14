//! Rebuild the layout of code from OCR results.
//!
//! OCR returns one box per recognized run of text, with no notion of
//! indentation. In a screenshot of code the font is monospaced, so the width
//! of one character can be estimated from the boxes themselves, and each
//! box's left edge converted into a column. That recovers indentation,
//! aligned columns, and blank lines.
//!
//! Coordinates are whatever the OCR engine reports, as long as x grows to the
//! right; the vertical direction is detected (see `rebuild`).

#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[values.len() / 2])
}

/// Typographic substitutions OCR likes to make that are never right in code.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{201C}' | '\u{201D}' | '\u{201E}' => out.push('"'),
            '\u{2018}' | '\u{2019}' | '\u{201A}' => out.push('\''),
            '\u{00A0}' => out.push(' '),
            // "->" and "=>" set in a coding font read as arrows.
            '\u{2192}' => out.push_str("->"),
            '\u{21D2}' => out.push_str("=>"),
            other => out.push(other),
        }
    }
    out
}

/// Window chrome in a screenshot (the three title-bar dots) comes back as a
/// line of bullets. It is never code.
fn is_decoration(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.chars().all(|c| matches!(c, '\u{2022}' | '\u{25CF}' | '\u{00B7}' | '\u{2219}' | '\u{26AB}' | ' '))
}

/// If nearly every indent sits on (or one column off) a multiple of 4 or 2,
/// snap to it: a one-column wobble from box-edge noise is far more likely
/// than a real 3- or 5-space indent.
fn snap_indents(indents: &mut [usize]) {
    let used: Vec<usize> = indents.iter().copied().filter(|&i| i > 0).collect();
    if used.is_empty() {
        return;
    }
    for unit in [4usize, 2] {
        let near = |i: usize| {
            let r = i % unit;
            r == 0 || r == 1 || r == unit - 1
        };
        let fits = used.iter().filter(|&&i| near(i)).count();
        // At unit 2 every number is "near"; require exact evens to dominate.
        let exact = used.iter().filter(|&&i| i % unit == 0).count();
        if fits * 10 >= used.len() * 9 && exact * 2 >= used.len() {
            for indent in indents.iter_mut() {
                if near(*indent) {
                    *indent = ((*indent + unit / 2) / unit) * unit;
                }
            }
            return;
        }
    }
}

pub fn rebuild(lines: &[Line]) -> String {
    let lines: Vec<&Line> = lines
        .iter()
        .filter(|l| !l.text.trim().is_empty() && !is_decoration(&l.text))
        .collect();
    if lines.is_empty() {
        return String::new();
    }

    // Width of one character cell, from every box that has enough text to
    // average out narrow and wide glyph boxes.
    let mut widths: Vec<f64> = lines
        .iter()
        .filter(|l| l.text.chars().count() >= 3 && l.width > 0.0)
        .map(|l| l.width / l.text.chars().count() as f64)
        .collect();
    let Some(cell) = median(&mut widths).filter(|c| *c > 0.0) else {
        return lines.iter().map(|l| normalize(&l.text)).collect::<Vec<_>>().join("\n");
    };
    let mut heights: Vec<f64> = lines.iter().map(|l| l.height).collect();
    let line_height = median(&mut heights).unwrap_or(0.0).max(f64::EPSILON);

    // Group boxes into rows by vertical centre.
    let mut sorted = lines.clone();
    sorted.sort_by(|a, b| {
        (a.y + a.height / 2.0)
            .partial_cmp(&(b.y + b.height / 2.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut rows: Vec<Vec<&Line>> = Vec::new();
    let mut row_centre = f64::NEG_INFINITY;
    for line in sorted {
        let centre = line.y + line.height / 2.0;
        if rows.is_empty() || (centre - row_centre).abs() > line_height * 0.5 {
            rows.push(vec![line]);
            row_centre = centre;
        } else {
            rows.last_mut().unwrap().push(line);
        }
    }

    // Some engines put the origin at the bottom-left, which would list the
    // rows bottom-up. The engine also reports boxes in reading order, so the
    // box it listed first tells us which end is the top.
    let first_reported = lines[0];
    let first_row_has_it = rows[0].iter().any(|l| std::ptr::eq(*l, first_reported));
    let last_row_has_it = rows[rows.len() - 1].iter().any(|l| std::ptr::eq(*l, first_reported));
    if !first_row_has_it && last_row_has_it {
        rows.reverse();
    }

    let left = lines.iter().map(|l| l.x).fold(f64::INFINITY, f64::min);
    let column = |x: f64| ((x - left) / cell).round().max(0.0) as usize;

    // Typical distance between consecutive rows, to spot blank lines.
    let centres: Vec<f64> = rows.iter().map(|r| r[0].y + r[0].height / 2.0).collect();
    let mut gaps: Vec<f64> = centres.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let pitch = median(&mut gaps.clone()).unwrap_or(line_height);
    gaps.push(0.0);

    let mut indents: Vec<usize> = rows
        .iter()
        .map(|r| column(r.iter().map(|l| l.x).fold(f64::INFINITY, f64::min)))
        .collect();
    snap_indents(&mut indents);

    let row_count = rows.len();
    let mut out = String::new();
    for (i, row) in rows.iter_mut().enumerate() {
        row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        let mut text = " ".repeat(indents[i]);
        let first_col = column(row[0].x);
        for (j, piece) in row.iter().enumerate() {
            if j > 0 {
                // Later pieces on the row keep their distance from the first.
                let target = indents[i] + column(piece.x).saturating_sub(first_col);
                let have = text.chars().count();
                text.push_str(&" ".repeat(target.saturating_sub(have).max(1)));
            }
            text.push_str(normalize(&piece.text).trim_end());
        }
        out.push_str(text.trim_end());
        if i + 1 < row_count {
            out.push('\n');
            // A gap of about two line pitches is one blank line, and so on.
            if pitch > 0.0 {
                let blanks = ((gaps[i] / pitch).round() as isize - 1).clamp(0, 3);
                for _ in 0..blanks {
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay text out on an 8x20 character grid, the way a screenshot would.
    fn boxes(code: &str, flip_y: bool) -> Vec<Line> {
        let rows: Vec<&str> = code.split('\n').collect();
        let total = rows.len() as f64;
        rows.iter()
            .enumerate()
            .filter(|(_, r)| !r.trim().is_empty())
            .map(|(i, r)| {
                let indent = r.len() - r.trim_start().len();
                let text = r.trim();
                // +-1.5px of noise on the left edge, as real boxes have
                let noise = if i % 2 == 0 { 1.5 } else { -1.5 };
                let y = if flip_y { (total - 1.0 - i as f64) * 20.0 } else { i as f64 * 20.0 };
                Line {
                    text: text.to_string(),
                    x: 40.0 + indent as f64 * 8.0 + noise,
                    y,
                    width: text.chars().count() as f64 * 8.0,
                    height: 16.0,
                }
            })
            .collect()
    }

    const SAMPLE: &str = "fn main() {\n    let v = vec![1, 2, 3];\n    for x in v {\n        println!(\"{}\", x);\n    }\n}\n\nfn other() {}";

    #[test]
    fn recovers_indentation_and_blank_lines() {
        assert_eq!(rebuild(&boxes(SAMPLE, false)), SAMPLE);
    }

    #[test]
    fn handles_bottom_left_origin() {
        assert_eq!(rebuild(&boxes(SAMPLE, true)), SAMPLE);
    }

    #[test]
    fn keeps_two_space_indents() {
        let code = "items:\n  - name: a\n    tags:\n      - x\n  - name: b";
        assert_eq!(rebuild(&boxes(code, false)), code);
    }

    #[test]
    fn joins_pieces_on_one_row_with_their_spacing() {
        let lines = vec![
            Line { text: "let x = 1;".into(), x: 0.0, y: 0.0, width: 80.0, height: 16.0 },
            Line { text: "// note".into(), x: 120.0, y: 1.0, width: 56.0, height: 16.0 },
        ];
        assert_eq!(rebuild(&lines), "let x = 1;     // note");
    }

    #[test]
    fn straightens_typographic_quotes() {
        let lines = vec![Line { text: "print(\u{201C}hi\u{201D})".into(), x: 0.0, y: 0.0, width: 88.0, height: 16.0 }];
        assert_eq!(rebuild(&lines), "print(\"hi\")");
    }

    #[test]
    fn drops_window_dots_and_restores_arrows() {
        let lines = vec![
            Line { text: "\u{2022} \u{2022}".into(), x: 0.0, y: 0.0, width: 24.0, height: 16.0 },
            Line { text: "fn f() \u{2192} u8 {".into(), x: 0.0, y: 40.0, width: 112.0, height: 16.0 },
        ];
        assert_eq!(rebuild(&lines), "fn f() -> u8 {");
    }

    #[test]
    fn empty_input() {
        assert_eq!(rebuild(&[]), "");
    }
}

#[cfg(test)]
mod real_image {
    /// Run against a real screenshot: OCR_CODE_IMAGE=/path/to.png cargo test real_image -- --ignored --nocapture
    #[test]
    #[ignore]
    fn print_rebuilt_code() {
        let path = std::env::var("OCR_CODE_IMAGE").expect("set OCR_CODE_IMAGE");
        let results = loupe_rs::recognize_text_with(std::path::Path::new(&path), loupe_rs::OcrOptions { language_correction: false }).expect("ocr");
        for r in results.iter().take(3) {
            eprintln!("box x={:.3} y={:.3} w={:.3} h={:.3} {:?}", r.x, r.y, r.width, r.height, r.text);
        }
        let lines: Vec<super::Line> = results
            .iter()
            .map(|r| super::Line { text: r.text.clone(), x: r.x, y: r.y, width: r.width, height: r.height })
            .collect();
        println!("-----\n{}\n-----", super::rebuild(&lines));
    }
}
