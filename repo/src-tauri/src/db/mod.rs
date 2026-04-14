//! Database layer — SQLite connection, encryption, masking, repos.

pub mod connection;
pub mod encryption;
pub mod masking;
pub mod repos;

pub use encryption::{CipherError, EncryptedField, FieldCipher};
pub use masking::{mask_national_id, mask_tail};
