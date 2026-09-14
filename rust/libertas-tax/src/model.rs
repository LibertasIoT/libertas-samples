use crate::FederalDraft;
use libertas_macros::{LibertasAvroDecode, LibertasAvroEncode, LibertasExport};

/// Saved tax return
/// Complete accepted pages; unfinished editor values are never stored.
#[derive(Clone, Debug, PartialEq, Eq, LibertasAvroEncode, LibertasAvroDecode, LibertasExport)]
pub enum TaxAppData {
    /// Accepted 2026 return
    Draft {
        /// Accepted answers
        draft: FederalDraft,
    },
}
