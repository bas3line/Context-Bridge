use std::{fmt::Display, fmt::Write};

/// Render untrusted text without allowing it to emit terminal control
/// sequences. JSON output is escaped by Serde; human output goes through this
/// helper before it reaches a controlling terminal.
#[must_use]
pub fn terminal_safe(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => output.push('\n'),
            '\r' => output.push_str("\\r"),
            '\t' => output.push('\t'),
            character if character.is_control() => {
                write!(output, "\\u{{{:04X}}}", character as u32)
                    .expect("writing to a String cannot fail");
            }
            character => output.push(character),
        }
    }
    output
}

/// Render untrusted text for a single table cell or field value.
#[must_use]
pub fn terminal_single_line(value: &str) -> String {
    terminal_safe(value)
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let headers = headers
        .iter()
        .map(|header| terminal_single_line(header))
        .collect::<Vec<_>>();
    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| terminal_single_line(value))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut widths: Vec<usize> = headers.iter().map(String::len).collect();
    for row in &rows {
        for (index, value) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(value.len());
            }
        }
    }
    println!(
        "{}",
        headers
            .iter()
            .enumerate()
            .map(|(index, header)| format!("{header:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in &rows {
        println!(
            "{}",
            row.iter()
                .enumerate()
                .map(|(index, value)| format!("{value:<width$}", width = widths[index]))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
}

pub fn print_field(label: &str, value: impl Display) {
    println!(
        "{}: {}",
        terminal_single_line(label),
        terminal_single_line(&value.to_string())
    );
}

#[cfg(test)]
mod tests {
    use super::terminal_safe;

    #[test]
    fn terminal_control_sequences_are_rendered_visibly() {
        let value = "safe\u{1b}]52;c;clipboard\u{7}";
        let safe = terminal_safe(value);
        assert!(!safe.contains('\u{1b}'));
        assert!(safe.contains("\\u{001B}]52;c;clipboard\\u{0007}"));
        assert_eq!(terminal_safe("first\nsecond"), "first\nsecond");
    }
}
