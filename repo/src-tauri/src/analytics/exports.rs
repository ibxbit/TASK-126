//! CSV and JSON exporters.
//!
//! Both are pure functions over an iterator of generic rows
//! (`serde_json::Value` objects), so the same call site can serve any
//! query shape — funnel results, retention cohorts, raw events, etc.
//! No I/O: the caller writes the returned String to a file (or hands
//! it to the WebView for download).

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExportError {
    #[error("rows are not all JSON objects")]
    NotObjectShape,
}

/// CSV (RFC 4180-ish, CRLF lines, double-quoted when needed).
///
/// Header row is the union of keys preserved in first-seen order.
/// Missing fields render as empty cells; nested objects/arrays are
/// serialized as compact JSON in the cell.
pub fn to_csv<I>(rows: I) -> Result<String, ExportError>
where
    I: IntoIterator<Item = Value>,
{
    let rows: Vec<Value> = rows.into_iter().collect();
    if rows.is_empty() {
        return Ok(String::new());
    }

    // Build header ordering deterministically.
    let mut headers: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &rows {
        let obj = r.as_object().ok_or(ExportError::NotObjectShape)?;
        for k in obj.keys() {
            if seen.insert(k.clone()) {
                headers.push(k.clone());
            }
        }
    }

    let mut out = String::new();
    out.push_str(&join_csv(&headers));
    out.push_str("\r\n");

    for r in &rows {
        let obj = r.as_object().ok_or(ExportError::NotObjectShape)?;
        let cells: Vec<String> = headers
            .iter()
            .map(|h| match obj.get(h) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.clone(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Number(n)) => n.to_string(),
                Some(other) => other.to_string(),
            })
            .collect();
        out.push_str(&join_csv_quoted(&cells));
        out.push_str("\r\n");
    }
    Ok(out)
}

fn join_csv(values: &[String]) -> String {
    values
        .iter()
        .map(|v| escape_csv(v))
        .collect::<Vec<_>>()
        .join(",")
}

fn join_csv_quoted(values: &[String]) -> String {
    values
        .iter()
        .map(|v| escape_csv(v))
        .collect::<Vec<_>>()
        .join(",")
}

/// Wrap in quotes and double-up internal quotes if the value contains
/// a comma, quote, CR or LF.
fn escape_csv(s: &str) -> String {
    let needs_quote = s.contains([',', '"', '\r', '\n']);
    if !needs_quote {
        return s.to_string();
    }
    let escaped = s.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// JSON Lines (newline-delimited JSON). One row per line, no leading
/// or trailing brackets — well suited to streaming and to `jq` style
/// post-processing.
pub fn to_json_lines<I>(rows: I) -> Result<String, ExportError>
where
    I: IntoIterator<Item = Value>,
{
    let mut out = String::new();
    for r in rows {
        match serde_json::to_string(&r) {
            Ok(line) => {
                out.push_str(&line);
                out.push('\n');
            }
            Err(_) => return Err(ExportError::NotObjectShape),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn csv_unions_keys_in_first_seen_order() {
        let rows = vec![
            json!({"a": 1, "b": 2}),
            json!({"b": 3, "c": 4}),
        ];
        let csv = to_csv(rows).unwrap();
        let mut lines = csv.split("\r\n");
        assert_eq!(lines.next(), Some("a,b,c"));
        assert_eq!(lines.next(), Some("1,2,"));
        assert_eq!(lines.next(), Some(",3,4"));
    }

    #[test]
    fn csv_quotes_and_escapes() {
        let rows = vec![json!({"x": "a,b", "y": "she said \"hi\"", "z": "line1\nline2"})];
        let csv = to_csv(rows).unwrap();
        let body = csv.trim_end_matches("\r\n").split("\r\n").nth(1).unwrap().to_string();
        assert!(body.contains("\"a,b\""));
        assert!(body.contains("\"she said \"\"hi\"\"\""));
        assert!(body.contains("\"line1\nline2\""));
    }

    #[test]
    fn csv_handles_empty_input() {
        assert_eq!(to_csv(Vec::<Value>::new()).unwrap(), "");
    }

    #[test]
    fn jsonl_emits_one_line_per_row() {
        let rows = vec![json!({"a":1}), json!({"b":2})];
        let s = to_json_lines(rows).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], r#"{"a":1}"#);
        assert_eq!(lines[1], r#"{"b":2}"#);
    }

    #[test]
    fn csv_rejects_non_object_rows() {
        assert!(matches!(
            to_csv(vec![json!([1,2,3])]).unwrap_err(),
            ExportError::NotObjectShape
        ));
    }
}
