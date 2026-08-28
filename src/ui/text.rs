use unicode_width::UnicodeWidthChar;

pub(crate) fn sanitize_inline(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\t' => sanitized.push_str("    "),
            '\n' => sanitized.push_str("\\n"),
            '\r' => sanitized.push_str("\\r"),
            character if character.is_control() => {
                sanitized.extend(character.escape_default());
            }
            character => sanitized.push(character),
        }
    }
    sanitized
}

pub(crate) fn truncate(text: &str, width: usize) -> String {
    let sanitized = sanitize_inline(text);
    if display_width(&sanitized) <= width {
        return sanitized;
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    let content_width = width - 3;
    let mut result = take_width(&sanitized, content_width);
    result.push_str("...");
    result
}

pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let sanitized = sanitize_inline(text);
    if width == 0 || sanitized.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for character in sanitized.chars() {
        let character_width = character.width().unwrap_or(0);
        if current_width > 0 && current_width + character_width > width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        if character_width > width {
            continue;
        }
        current.push(character);
        current_width += character_width;
    }
    lines.push(current);
    lines
}

pub(crate) fn display_width(text: &str) -> usize {
    text.chars()
        .map(|character| character.width().unwrap_or(0))
        .sum()
}

fn take_width(text: &str, width: usize) -> String {
    let mut result = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width {
            break;
        }
        result.push(character);
        used += character_width;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_by_terminal_columns() {
        assert_eq!(truncate("中文路径.rs", 9), "中文路...");
        assert_eq!(display_width(&truncate("中文路径.rs", 9)), 9);
    }

    #[test]
    fn wraps_wide_text_without_splitting_characters() {
        assert_eq!(wrap("甲乙丙丁", 5), vec!["甲乙", "丙丁"]);
    }

    #[test]
    fn makes_terminal_controls_visible() {
        assert_eq!(sanitize_inline("a\u{1b}[2J\tb"), "a\\u{1b}[2J    b");
        assert_eq!(sanitize_inline("a\nb\rc"), "a\\nb\\rc");
    }
}
