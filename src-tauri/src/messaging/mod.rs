//! Admin-to-hospital messaging system.
//!
//! Receives messages from cloud admins via the `hospital/{code}/inbox` Firestore subcollection.

pub mod types;
pub mod service;
pub mod files;
pub mod file_actions;
