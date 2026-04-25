use regex::Regex;

pub fn clean_output(raw: &str) -> String {
    let ansi = Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("valid ansi regex");
    let mut text = raw.replace("\r\n", "\n").replace('\r', "\n");
    text = ansi.replace_all(&text, "").to_string();

    let mut lines: Vec<&str> = text.lines().collect();
    while matches!(lines.last(), Some(last) if last.trim() == ">" || last.trim().is_empty()) {
        lines.pop();
    }

    lines.join("\n").trim().to_string()
}
