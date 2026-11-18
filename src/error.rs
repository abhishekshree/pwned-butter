use thiserror::Error;

/// An expected business rejection, not an operational failure.
///
/// Fieldless so every failure reason is a typed variant rather than a string
/// in disguise. `thiserror` gives each variant a human-readable `Display`
/// message for the run log, and a small `Error` impl for the one fallible
/// boundary that uses it. No other error path in the crate is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Rejection {
    #[error("city is outside the covered area")]
    OutOfScopeCity,
    #[error("no establishment named")]
    NoEstablishment,
    #[error("no reported or publication date")]
    NoActionDate,
    #[error("quote-led establishment")]
    QuoteLedEstablishment,
    #[error("no concrete action yet")]
    PendingAction,
    #[error("establishment is a product, not an outlet")]
    ProductNotOutlet,
    #[error("establishment is the regulator, not an outlet")]
    RegulatorNotOutlet,
    #[error("establishment reads like a headline fragment")]
    HeadlineFragment,
    #[error("no substantive details")]
    NoSubstantiveDetails,
}
