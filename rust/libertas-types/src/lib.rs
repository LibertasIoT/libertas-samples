#![forbid(unsafe_code)]

extern crate alloc;
use libertas_macros::*;

#[derive(Debug, PartialEq, Eq, Clone, LibertasAvroDecode, LibertasAvroEncode, LibertasExport)]
pub enum DayOfWeek {
    Sun,
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
}

