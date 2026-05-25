use std::borrow::Cow;

/// Make TOML 1.1 inline tables parseable by taplo 0.14 while preserving byte offsets.
pub(crate) fn normalize_multiline_inline_tables(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut normalized = None;
    let mut index = 0;
    let mut inline_table_stack = Vec::new();

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index = skip_basic_string(bytes, index);
            }
            b'\'' => {
                index = skip_literal_string(bytes, index);
            }
            b'#' => {
                if inline_table_stack.is_empty() {
                    index = skip_comment(bytes, index);
                } else {
                    index = replace_comment(bytes, &mut normalized, index);
                }
            }
            b'\r' | b'\n' if current_array_depth(&inline_table_stack) == Some(0) => {
                replace_byte(bytes, &mut normalized, index, b' ');
                index += 1;
            }
            b'{' => {
                if current_array_depth(&inline_table_stack) == Some(0) {
                    clear_current_comma(&mut inline_table_stack);
                }
                inline_table_stack.push(InlineTableFrame::default());
                index += 1;
            }
            b'}' if !inline_table_stack.is_empty() => {
                if current_array_depth(&inline_table_stack) == Some(0) {
                    if let Some(Some(comma_index)) =
                        inline_table_stack.last().map(|frame| frame.comma_index)
                    {
                        replace_byte(bytes, &mut normalized, comma_index, b' ');
                    }
                }

                inline_table_stack.pop();
                if current_array_depth(&inline_table_stack) == Some(0) {
                    // The parent comma was cleared before pushing this nested inline table.
                    // Clearing again is normally a no-op, but treats `}` like other parent bytes.
                    clear_current_comma(&mut inline_table_stack);
                }
                index += 1;
            }
            b'[' if !inline_table_stack.is_empty() => {
                if current_array_depth(&inline_table_stack) == Some(0) {
                    clear_current_comma(&mut inline_table_stack);
                }
                if let Some(frame) = inline_table_stack.last_mut() {
                    frame.array_depth += 1;
                }
                index += 1;
            }
            b']' if !inline_table_stack.is_empty() => {
                if let Some(frame) = inline_table_stack.last_mut() {
                    frame.array_depth = frame.array_depth.saturating_sub(1);
                }
                index += 1;
            }
            b',' if current_array_depth(&inline_table_stack) == Some(0) => {
                if let Some(frame) = inline_table_stack.last_mut() {
                    frame.comma_index = Some(index);
                }
                index += 1;
            }
            byte if current_array_depth(&inline_table_stack) == Some(0)
                && !byte.is_ascii_whitespace() =>
            {
                // Multi-byte UTF-8 bytes are non-ASCII and can harmlessly retrigger this arm.
                clear_current_comma(&mut inline_table_stack);
                index += 1;
            }
            _ => {
                index += 1;
            }
        }
    }

    let result = if let Some(normalized) = normalized {
        Cow::Owned(String::from_utf8(normalized).expect("normalization preserves UTF-8"))
    } else {
        Cow::Borrowed(text)
    };

    debug_assert_eq!(result.len(), text.len());
    result
}

#[derive(Default)]
struct InlineTableFrame {
    array_depth: usize,
    comma_index: Option<usize>,
}

fn current_array_depth(inline_table_stack: &[InlineTableFrame]) -> Option<usize> {
    inline_table_stack.last().map(|frame| frame.array_depth)
}

fn clear_current_comma(inline_table_stack: &mut [InlineTableFrame]) {
    if let Some(frame) = inline_table_stack.last_mut() {
        frame.comma_index = None;
    }
}

fn skip_comment(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\r' && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn replace_comment(bytes: &[u8], normalized: &mut Option<Vec<u8>>, mut index: usize) -> usize {
    // A brace after `#` is comment text, not an inline-table delimiter.
    while index < bytes.len() && bytes[index] != b'\r' && bytes[index] != b'\n' {
        replace_byte(bytes, normalized, index, b' ');
        index += 1;
    }
    index
}

fn replace_byte(bytes: &[u8], normalized: &mut Option<Vec<u8>>, index: usize, byte: u8) {
    normalized.get_or_insert_with(|| bytes.to_vec())[index] = byte;
}

fn skip_basic_string(bytes: &[u8], index: usize) -> usize {
    if starts_with(bytes, index, b"\"\"\"") {
        skip_basic_multiline_string(bytes, index + 3)
    } else {
        skip_basic_single_line_string(bytes, index + 1)
    }
}

fn skip_basic_single_line_string(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index = (index + 2).min(bytes.len());
            }
            b'"' => {
                return index + 1;
            }
            _ => {
                index += 1;
            }
        }
    }
    index
}

fn skip_basic_multiline_string(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index = (index + 2).min(bytes.len());
            }
            b'"' if starts_with(bytes, index, b"\"\"\"") => {
                // TOML 1.0 permits up to two quote characters immediately
                // preceding the closing delimiter to be part of the content
                // (e.g. `""""quoted""""`). Without consuming them, the outer
                // scanner would treat the stray quote as opening a new string.
                return consume_trailing_quotes(bytes, index + 3, b'"');
            }
            _ => {
                index += 1;
            }
        }
    }
    index
}

fn skip_literal_string(bytes: &[u8], index: usize) -> usize {
    if starts_with(bytes, index, b"'''") {
        skip_literal_multiline_string(bytes, index + 3)
    } else {
        skip_literal_single_line_string(bytes, index + 1)
    }
}

fn skip_literal_single_line_string(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn skip_literal_multiline_string(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        if starts_with(bytes, index, b"'''") {
            return consume_trailing_quotes(bytes, index + 3, b'\'');
        }
        index += 1;
    }
    index
}

fn consume_trailing_quotes(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    let limit = (index + 2).min(bytes.len());
    while index < limit && bytes[index] == quote {
        index += 1;
    }
    index
}

fn starts_with(bytes: &[u8], index: usize, pattern: &[u8]) -> bool {
    bytes
        .get(index..index.saturating_add(pattern.len()))
        .is_some_and(|candidate| candidate == pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_multiline_inline_table_without_moving_offsets() {
        let toml = concat!(
            "[dependencies]\n",
            "clap = {\n",
            "  workspace = true,\n",
            "  features = [\n",
            "    \"derive\",\n",
            "  ],\n",
            "}\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert_eq!(
            normalized.as_ref(),
            concat!(
                "[dependencies]\n",
                "clap = { ",
                "  workspace = true, ",
                "  features = [\n",
                "    \"derive\",\n",
                "  ]  ",
                "}\n",
            )
        );
    }

    #[test]
    fn removes_comments_inside_inline_table() {
        let toml = concat!(
            "[dependencies]\n",
            "clap = { # inherited\n",
            "  workspace = true, # local features\n",
            "  features = [\"derive\"],\n",
            "}\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(!normalized.as_ref().contains("inherited"));
        assert!(!normalized.as_ref().contains("local features"));
        assert!(normalized.as_ref().contains("workspace = true"));
    }

    #[test]
    fn removes_inline_table_comment_that_reaches_eof() {
        let toml = concat!(
            "[dependencies]\n",
            "clap = {\n",
            "  workspace = true # trailing comment"
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(!normalized.as_ref().contains("trailing comment"));
        assert!(normalized.as_ref().contains("workspace = true"));
    }

    #[test]
    fn removes_multibyte_comments_without_breaking_utf8() {
        let toml = concat!(
            "[dependencies]\n",
            "clap = {\n",
            "  version = \"1\", # 注释\n",
            "}\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(std::str::from_utf8(normalized.as_ref().as_bytes()).is_ok());
        assert!(!normalized.as_ref().contains("注释"));
    }

    #[test]
    fn preserves_string_contents_inside_inline_table() {
        let toml = concat!(
            "[dependencies]\n",
            "example = {\n",
            "  version = \"1.0 # not a comment\",\n",
            "  path = '''multi\n",
            "line # not a comment''',\n",
            "}\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(normalized.as_ref().contains("\"1.0 # not a comment\""));
        assert!(normalized.as_ref().contains("line # not a comment"));
    }

    #[test]
    fn normalizes_nested_inline_table_inside_array() {
        let toml = concat!(
            "entries = [\n",
            "  {\n",
            "    name = \"first\",\n",
            "    details = { enabled = true,\n",
            "      note = \"ok\",\n",
            "    },\n",
            "  },\n",
            "]\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(taplo::parser::parse(&normalized).errors.is_empty());
    }

    #[test]
    fn normalizes_inline_table_after_multiline_string_with_inside_quotes() {
        // `""""quoted""""` is a valid TOML 1.0 multi-line basic string with a
        // leading and trailing quote inside the delimiters. The scanner used to
        // stop after the first internal `"""`, leaving a stray `"` that swallowed
        // later content and prevented the inline table below from being
        // normalized.
        let toml = concat!(
            "[package]\n",
            "description = \"\"\"\"quoted\"\"\"\"\n",
            "[dependencies]\n",
            "clap = {\n",
            "  workspace = true,\n",
            "  features = [\"derive\"],\n",
            "}\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(normalized.as_ref().contains("\"\"\"\"quoted\"\"\"\""));
        assert!(taplo::parser::parse(&normalized).errors.is_empty());
    }

    #[test]
    fn normalizes_inline_table_after_literal_multiline_with_inside_quotes() {
        let toml = concat!(
            "[package]\n",
            "description = ''''quoted''''\n",
            "[dependencies]\n",
            "clap = {\n",
            "  workspace = true,\n",
            "  features = [\"derive\"],\n",
            "}\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert_eq!(normalized.len(), toml.len());
        assert!(normalized.as_ref().contains("''''quoted''''"));
        assert!(taplo::parser::parse(&normalized).errors.is_empty());
    }

    #[test]
    fn multiline_strings_consume_up_to_two_trailing_quotes() {
        // Five consecutive quotes at the close: two inside-quotes plus the
        // three-quote closer. Anything more would be invalid TOML.
        let basic = "x = \"\"\"\"\"abc\"\"\"\"\"\n";
        let normalized = normalize_multiline_inline_tables(basic);
        assert_eq!(normalized.as_ref(), basic);
        assert!(taplo::parser::parse(&normalized).errors.is_empty());

        let literal = "x = '''''abc'''''\n";
        let normalized = normalize_multiline_inline_tables(literal);
        assert_eq!(normalized.as_ref(), literal);
        assert!(taplo::parser::parse(&normalized).errors.is_empty());
    }

    #[test]
    fn leaves_standard_toml_unchanged() {
        let toml = concat!(
            "[dependencies]\n",
            "serde = { version = \"1\", features = [\"derive\"] }\n",
            "tokio = \"1\"\n",
        );

        let normalized = normalize_multiline_inline_tables(toml);

        assert!(matches!(normalized, Cow::Borrowed(_)));
        assert_eq!(normalized.as_ref(), toml);
    }
}
