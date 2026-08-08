//! Libertas shared types
//! Defines small reusable data types for Libertas applications without an
//! executable application entry point.
//! #[libertas_types_only]
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Day of week
/// Identifies one calendar weekday, ordered from Sunday through Saturday.
#[derive(Debug, PartialEq, Eq, Clone, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum DayOfWeek {
    /// Sunday
    Sun,
    /// Monday
    Mon,
    /// Tuesday
    Tue,
    /// Wednesday
    Wed,
    /// Thursday
    Thu,
    /// Friday
    Fri,
    /// Saturday
    Sat,
}
