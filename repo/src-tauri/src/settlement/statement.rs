//! Settlement statement generator.
//!
//! Pure functions over already-fetched data: the caller assembles
//! `SettlementStatement` inputs from the repositories, hands them
//! here, and gets back either a structured statement (for the React
//! UI) or a printable HTML document (for the Print Preview window).
//!
//! No I/O, no dependencies on Tauri — fully testable.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum StatementError {
    #[error("deductions ({deductions}¢) exceed total deposit ({deposit}¢)")]
    OverDeducted { deposit: i64, deductions: i64 },

    #[error("currency mismatch across deposits ({0})")]
    MixedCurrency(String),

    #[error("at least one deposit is required to generate a statement")]
    NoDeposit,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DepositInput {
    pub id: Uuid,
    pub amount_cents: i64,
    pub currency: String,
    pub received_at_unix: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeductionLine {
    pub id: Uuid,
    pub category: String,
    pub description: String,
    pub amount_cents: i64,
    pub evidence_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatementInputs {
    pub settlement_id: Uuid,
    pub tenant_id: Uuid,
    pub case_number: String,
    pub resident_display_name: String,
    pub unit_label: Option<String>,
    pub move_out_date_unix: i64,
    pub deposits: Vec<DepositInput>,
    pub deductions: Vec<DeductionLine>,
    pub generated_at_unix: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettlementStatement {
    pub settlement_id: Uuid,
    pub tenant_id: Uuid,
    pub case_number: String,
    pub resident_display_name: String,
    pub unit_label: Option<String>,
    pub move_out_date_unix: i64,
    pub generated_at_unix: i64,
    pub currency: String,
    pub deposit_total_cents: i64,
    pub deductions: Vec<DeductionLine>,
    pub deductions_total_cents: i64,
    pub refund_cents: i64,
    /// Same numbers formatted as "$1,234.56" for direct UI rendering.
    pub display: StatementDisplay,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatementDisplay {
    pub deposit_total: String,
    pub deductions_total: String,
    pub refund: String,
}

/// Build the statement from raw inputs. Validates currency
/// homogeneity and that deductions never exceed the deposit total.
pub fn generate_statement(
    inputs: StatementInputs,
) -> Result<SettlementStatement, StatementError> {
    if inputs.deposits.is_empty() {
        return Err(StatementError::NoDeposit);
    }
    let currency = inputs.deposits[0].currency.clone();
    for d in &inputs.deposits {
        if d.currency != currency {
            return Err(StatementError::MixedCurrency(format!(
                "{} vs {}",
                currency, d.currency
            )));
        }
    }

    let deposit_total: i64 = inputs.deposits.iter().map(|d| d.amount_cents).sum();
    let deductions_total: i64 = inputs.deductions.iter().map(|d| d.amount_cents).sum();

    if deductions_total > deposit_total {
        return Err(StatementError::OverDeducted {
            deposit: deposit_total,
            deductions: deductions_total,
        });
    }
    let refund = deposit_total - deductions_total;

    let display = StatementDisplay {
        deposit_total: format_money(deposit_total, &currency),
        deductions_total: format_money(deductions_total, &currency),
        refund: format_money(refund, &currency),
    };

    Ok(SettlementStatement {
        settlement_id: inputs.settlement_id,
        tenant_id: inputs.tenant_id,
        case_number: inputs.case_number,
        resident_display_name: inputs.resident_display_name,
        unit_label: inputs.unit_label,
        move_out_date_unix: inputs.move_out_date_unix,
        generated_at_unix: inputs.generated_at_unix,
        currency,
        deposit_total_cents: deposit_total,
        deductions: inputs.deductions,
        deductions_total_cents: deductions_total,
        refund_cents: refund,
        display,
    })
}

/// Render a printable HTML document. Self-contained (inline styles,
/// no external assets) so the WebView can print it offline.
pub fn render_statement_html(s: &SettlementStatement) -> String {
    let unit = s.unit_label.as_deref().unwrap_or("—");
    let mut rows = String::new();
    for d in &s.deductions {
        rows.push_str(&format!(
            "<tr>\
                <td>{cat}</td>\
                <td>{desc}</td>\
                <td class=\"num\">{amt}</td>\
                <td class=\"num\">{ev}</td>\
            </tr>",
            cat = html_escape(&d.category),
            desc = html_escape(&d.description),
            amt = html_escape(&format_money(d.amount_cents, &s.currency)),
            ev = d.evidence_count,
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"4\" class=\"empty\">No deductions</td></tr>");
    }

    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Settlement Statement — {case}</title>
<style>
  body {{ font-family: 'Segoe UI', Arial, sans-serif; color: #111; margin: 32px; }}
  h1 {{ font-size: 18pt; margin-bottom: 4px; }}
  .meta {{ font-size: 10pt; color: #555; margin-bottom: 24px; }}
  table {{ width: 100%; border-collapse: collapse; }}
  th, td {{ border: 1px solid #ccc; padding: 6px 10px; font-size: 10pt; }}
  th {{ background: #f3f3f3; text-align: left; }}
  td.num {{ text-align: right; font-variant-numeric: tabular-nums; }}
  td.empty {{ text-align: center; color: #888; }}
  .totals {{ margin-top: 24px; width: 60%; margin-left: auto; }}
  .totals tr td:first-child {{ font-weight: 600; }}
  .refund td {{ font-size: 12pt; background: #f7fbff; }}
</style></head>
<body>
  <h1>Settlement Statement</h1>
  <div class="meta">
    Case <b>{case}</b> &middot; Resident <b>{res}</b> &middot; Unit <b>{unit}</b><br>
    Move-out: {mo} &middot; Generated: {gen}
  </div>

  <table>
    <thead><tr>
      <th>Category</th><th>Description</th>
      <th class="num">Amount</th><th class="num">Evidence</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>

  <table class="totals">
    <tr><td>Deposit total</td>     <td class="num">{dep}</td></tr>
    <tr><td>Deductions total</td>  <td class="num">{ded}</td></tr>
    <tr class="refund"><td>Refund due to resident</td><td class="num">{ref_}</td></tr>
  </table>
</body></html>"#,
        case = html_escape(&s.case_number),
        res = html_escape(&s.resident_display_name),
        unit = html_escape(unit),
        mo = format_date_us(s.move_out_date_unix),
        gen = format_datetime_us(s.generated_at_unix),
        rows = rows,
        dep = html_escape(&s.display.deposit_total),
        ded = html_escape(&s.display.deductions_total),
        ref_ = html_escape(&s.display.refund),
    )
}

// ── Formatting helpers (offline, no external libs beyond chrono) ────────

fn format_money(cents: i64, currency: &str) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let frac = abs % 100;
    // Insert thousands separators.
    let mut whole = dollars.to_string();
    let mut out = String::new();
    let bytes = whole.as_bytes();
    for (i, b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    whole = out.chars().rev().collect();
    let prefix = match currency {
        "USD" => "$",
        _ => "",
    };
    let sign = if neg { "-" } else { "" };
    format!("{sign}{prefix}{whole}.{frac:02}")
}

fn format_date_us(unix: i64) -> String {
    use chrono::{Datelike, TimeZone, Utc};
    let dt = Utc.timestamp_opt(unix, 0).single().unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
    format!("{:02}/{:02}/{:04}", dt.month(), dt.day(), dt.year())
}

fn format_datetime_us(unix: i64) -> String {
    use chrono::{Datelike, TimeZone, Timelike, Utc};
    let dt = Utc.timestamp_opt(unix, 0).single().unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
    let h24 = dt.hour();
    let ampm = if h24 >= 12 { "PM" } else { "AM" };
    let h12 = match h24 % 12 { 0 => 12, h => h };
    format!("{:02}/{:02}/{:04} {:02}:{:02} {}", dt.month(), dt.day(), dt.year(), h12, dt.minute(), ampm)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(cents: i64, cur: &str) -> DepositInput {
        DepositInput {
            id: Uuid::new_v4(),
            amount_cents: cents,
            currency: cur.into(),
            received_at_unix: 0,
        }
    }
    fn ded(cents: i64) -> DeductionLine {
        DeductionLine {
            id: Uuid::new_v4(),
            category: "damage".into(),
            description: "wall repair".into(),
            amount_cents: cents,
            evidence_count: 1,
        }
    }
    fn inputs(deposits: Vec<DepositInput>, deductions: Vec<DeductionLine>) -> StatementInputs {
        StatementInputs {
            settlement_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            case_number: "MO-2026-0001".into(),
            resident_display_name: "Casey Lee".into(),
            unit_label: Some("4B".into()),
            move_out_date_unix: 0,
            deposits,
            deductions,
            generated_at_unix: 0,
        }
    }

    #[test]
    fn refund_is_deposit_minus_deductions() {
        let s = generate_statement(inputs(vec![dep(150_000, "USD")], vec![ded(40_000)])).unwrap();
        assert_eq!(s.deposit_total_cents, 150_000);
        assert_eq!(s.deductions_total_cents, 40_000);
        assert_eq!(s.refund_cents, 110_000);
        assert_eq!(s.display.refund, "$1,100.00");
    }

    #[test]
    fn over_deduction_rejected() {
        let err = generate_statement(inputs(vec![dep(50_000, "USD")], vec![ded(60_000)])).unwrap_err();
        assert!(matches!(err, StatementError::OverDeducted { .. }));
    }

    #[test]
    fn mixed_currency_rejected() {
        let err = generate_statement(inputs(vec![dep(100, "USD"), dep(100, "EUR")], vec![]))
            .unwrap_err();
        assert!(matches!(err, StatementError::MixedCurrency(_)));
    }

    #[test]
    fn no_deposit_rejected() {
        let err = generate_statement(inputs(vec![], vec![])).unwrap_err();
        assert!(matches!(err, StatementError::NoDeposit));
    }

    #[test]
    fn money_formatting_handles_thousands_and_negatives() {
        assert_eq!(format_money(0, "USD"), "$0.00");
        assert_eq!(format_money(7, "USD"), "$0.07");
        assert_eq!(format_money(123_456_789, "USD"), "$1,234,567.89");
        assert_eq!(format_money(-50_000, "USD"), "-$500.00");
    }

    #[test]
    fn html_renders_self_contained() {
        let s = generate_statement(inputs(vec![dep(100_000, "USD")], vec![ded(25_000)])).unwrap();
        let html = render_statement_html(&s);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("MO-2026-0001"));
        assert!(html.contains("$1,000.00")); // deposit
        assert!(html.contains("$250.00"));   // deduction line
        assert!(html.contains("$750.00"));   // refund
        // No external assets referenced.
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }

    #[test]
    fn html_escapes_user_text() {
        let mut i = inputs(vec![dep(100, "USD")], vec![]);
        i.resident_display_name = "Alice <script>".into();
        let s = generate_statement(i).unwrap();
        let html = render_statement_html(&s);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
