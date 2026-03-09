use rowan::TextRange;

/// Convert a TextRange to line/column (0-based) for error messages
pub fn text_range_to_line_col(text: &str, range: TextRange) -> (u32, u32) {
    let offset: usize = range.start().into();
    let mut line = 0u32;
    let mut col = 0u32;

    for (idx, ch) in text.chars().enumerate() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    (line, col)
}

/// Extract a snippet of code around a TextRange for error messages
pub fn extract_snippet(text: &str, range: TextRange, context_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (target_line, _) = text_range_to_line_col(text, range);

    let start_line = target_line.saturating_sub(context_lines as u32) as usize;
    let end_line = ((target_line + context_lines as u32 + 1) as usize).min(lines.len());

    let snippet_lines: Vec<String> = lines[start_line..end_line]
        .iter()
        .enumerate()
        .map(|(idx, line)| {
            let line_num = start_line + idx + 1;
            format!("{:4} | {}", line_num, line)
        })
        .collect();

    snippet_lines.join("\n")
}
