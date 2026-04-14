//! Parcel lifecycle states. Strings match the DB CHECK constraint in
//! `0002_parcel_state_machine.sql`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParcelState {
    CheckedIn,
    CheckedOut,
    Delivered,
    ReceiptConfirmed,
    ReturnedException,
}

/// Sentinel used by the genesis rule: the "from" side of the very
/// first transition that creates a parcel (moves it into CheckedIn).
pub const GENESIS: &str = "__genesis__";

impl ParcelState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ParcelState::CheckedIn => "checked_in",
            ParcelState::CheckedOut => "checked_out",
            ParcelState::Delivered => "delivered",
            ParcelState::ReceiptConfirmed => "receipt_confirmed",
            ParcelState::ReturnedException => "returned_exception",
        }
    }

    pub fn from_str(s: &str) -> Option<ParcelState> {
        match s {
            "checked_in" => Some(ParcelState::CheckedIn),
            "checked_out" => Some(ParcelState::CheckedOut),
            "delivered" => Some(ParcelState::Delivered),
            "receipt_confirmed" => Some(ParcelState::ReceiptConfirmed),
            "returned_exception" => Some(ParcelState::ReturnedException),
            _ => None,
        }
    }
}
