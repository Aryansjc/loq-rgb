//! loq-rgb — RGB keyboard controller for the Lenovo LOQ / Legion / IdeaPad
//! Gaming 4-zone ITE keyboard lighting (048d:c9xx family, CC/16 protocol).
//!
//! Library crate: all hardware-independent logic lives here so the GUI and
//! CLI binaries stay thin, and everything is unit-testable without the
//! device.

pub mod backend;
pub mod config;
pub mod controller;
pub mod daemon;
pub mod detect;
pub mod devices;
pub mod effects;
pub mod error;
pub mod flow;
pub mod gui;
pub mod hotkey;
pub mod instance;
pub mod model;
pub mod packet;
pub mod profiles;
