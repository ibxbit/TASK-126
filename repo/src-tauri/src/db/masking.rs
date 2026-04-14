//! Partial-masking helpers for sensitive identifiers.
//!
//! The masked string is the ONLY form ever rendered in lists, exports,
//! or logs. The full value is always stored encrypted (see
//! `encryption.rs`) and decrypted only for the narrow code path that
//! must see it (e.g. a detail view authorized by permission check).

/// Produce an SSN-style mask from a raw identifier.
///
/// - Non-digit characters are stripped first.
/// - All but the last 4 digits are replaced with `X`.
/// - Grouping matches the most common SSN layout when the input has
///   exactly 9 digits: `"XXX-XX-1234"`. For other lengths we return
///   `"****1234"`-style output.
///
/// Examples:
/// ```
/// # use shoreline::db::masking::mask_national_id;
/// assert_eq!(mask_national_id("123-45-6789"), "XXX-XX-6789");
/// assert_eq!(mask_national_id("12345678901"), "*******8901");
/// assert_eq!(mask_national_id("7"),           "*7");
/// ```
pub fn mask_national_id(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return String::new();
    }

    if digits.len() == 9 {
        // Standard SSN layout.
        let last4 = &digits[5..9];
        return format!("XXX-XX-{last4}");
    }

    // Generic fallback: keep last 4, mask the rest with '*'.
    if digits.len() <= 4 {
        return format!("*{}", &digits[digits.len().saturating_sub(1)..]);
    }
    let visible = &digits[digits.len() - 4..];
    let masked: String = "*".repeat(digits.len() - 4);
    format!("{masked}{visible}")
}

/// Mask all but the trailing `keep` characters of an arbitrary string.
/// Useful for account numbers, reference codes, tracking numbers that
/// should not be fully exposed in list views.
pub fn mask_tail(raw: &str, keep: usize) -> String {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() <= keep {
        return "*".repeat(chars.len());
    }
    let hidden = chars.len() - keep;
    let mut out = String::with_capacity(chars.len());
    for _ in 0..hidden {
        out.push('*');
    }
    out.extend(&chars[hidden..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_ssn() {
        assert_eq!(mask_national_id("123-45-6789"), "XXX-XX-6789");
        assert_eq!(mask_national_id("123456789"), "XXX-XX-6789");
    }

    #[test]
    fn non_standard_length() {
        assert_eq!(mask_national_id("12345678901"), "*******8901");
    }

    #[test]
    fn short_input() {
        assert_eq!(mask_national_id("7"), "*7");
        assert_eq!(mask_national_id(""), "");
    }

    #[test]
    fn tail_masking() {
        assert_eq!(mask_tail("TRACK-ABCDE12345", 4), "************2345");
        assert_eq!(mask_tail("abc", 4), "***");
    }

    #[test]
    fn ignores_non_digits() {
        assert_eq!(mask_national_id("SSN: 111-22-3333!"), "XXX-XX-3333");
    }
}
