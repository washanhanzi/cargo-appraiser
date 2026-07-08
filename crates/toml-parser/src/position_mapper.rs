use ls_types::{Position, Range};
use unicode_segmentation::UnicodeSegmentation;

/// Converts tombi text positions to LSP positions.
///
/// tombi counts columns in grapheme clusters; the LSP protocol counts them
/// in UTF-16 code units. The two agree on pure-ASCII lines, and diverge as
/// soon as a line contains multi-byte characters (emoji, CJK, combining
/// marks). Lines are split the same way tombi does: "\n" or "\r\n".
pub(crate) struct PositionMapper<'a> {
    lines: Vec<&'a str>,
}

impl<'a> PositionMapper<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            lines: text
                .split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l))
                .collect(),
        }
    }

    pub fn position(&self, pos: tombi_text::Position) -> Position {
        let Some(line) = self.lines.get(pos.line as usize) else {
            return Position::new(pos.line, pos.column);
        };
        if line.is_ascii() {
            // Grapheme count == UTF-16 count on ASCII lines.
            return Position::new(pos.line, pos.column.min(line.len() as u32));
        }
        let mut utf16 = 0u32;
        for (i, grapheme) in line.graphemes(true).enumerate() {
            if i as u32 >= pos.column {
                break;
            }
            utf16 += grapheme.chars().map(char::len_utf16).sum::<usize>() as u32;
        }
        Position::new(pos.line, utf16)
    }

    pub fn range(&self, range: tombi_text::Range) -> Range {
        Range {
            start: self.position(range.start),
            end: self.position(range.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tombi_pos(line: u32, column: u32) -> tombi_text::Position {
        tombi_text::Position::new(line, column)
    }

    #[test]
    fn ascii_is_identity() {
        let mapper = PositionMapper::new("serde = \"1.0\"\ntokio = \"1\"");
        assert_eq!(mapper.position(tombi_pos(0, 5)), Position::new(0, 5));
        assert_eq!(mapper.position(tombi_pos(1, 8)), Position::new(1, 8));
    }

    #[test]
    fn emoji_counts_two_utf16_units() {
        // "👍" is one grapheme but two UTF-16 code units.
        let mapper = PositionMapper::new("# 👍 ok\nserde = \"1\"");
        // Column 2 (the emoji) → UTF-16 offset 2.
        assert_eq!(mapper.position(tombi_pos(0, 2)), Position::new(0, 2));
        // Column 3 (after the emoji) → UTF-16 offset 4.
        assert_eq!(mapper.position(tombi_pos(0, 3)), Position::new(0, 4));
        // The next line is unaffected.
        assert_eq!(mapper.position(tombi_pos(1, 5)), Position::new(1, 5));
    }

    #[test]
    fn cjk_counts_one_utf16_unit() {
        // CJK chars are one grapheme and one UTF-16 code unit (BMP).
        let mapper = PositionMapper::new("desc = \"你好\" # x");
        assert_eq!(mapper.position(tombi_pos(0, 10)), Position::new(0, 10));
    }

    #[test]
    fn crlf_lines() {
        let mapper = PositionMapper::new("a = 1\r\nb = 2\r\n");
        assert_eq!(mapper.position(tombi_pos(1, 4)), Position::new(1, 4));
    }
}
