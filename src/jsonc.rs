//! JSON with comments and trailing commas, as VS Code's files have it
//! (`devcontainer.json`, `.code-workspace`).

/// Parses `text`, leaving out `//` and `/* */` comments and commas before a
/// closing bracket.
pub fn parse(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&strip(text)).ok()
}

fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => while chars.next_if(|&c| c != '\n').is_some() {},
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    // Trailing commas: a comma followed (past white space) by } or ].
    let mut cleaned = String::with_capacity(out.len());
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in out.char_indices() {
        if in_string {
            in_string = !(c == '"' && !escaped);
            escaped = c == '\\' && !escaped;
            cleaned.push(c);
            continue;
        }
        if c == '"' {
            in_string = true;
        }
        let trailing = c == ',' && out[i + 1..].trim_start().starts_with(['}', ']']);
        if !trailing {
            cleaned.push(c);
        }
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_trailing_commas() {
        let text = r#"{
            // A comment, with a "quote"
            "name": "app", /* and another */
            "url": "https://example.com/a//b",
            "list": [1, 2, ],
            "text": "a, ]",
        }"#;
        let value = parse(text).unwrap();
        assert_eq!(value["name"], "app");
        assert_eq!(value["url"], "https://example.com/a//b");
        assert_eq!(value["list"], serde_json::json!([1, 2]));
        assert_eq!(value["text"], "a, ]");
        assert!(parse("{ nope").is_none());
    }
}
