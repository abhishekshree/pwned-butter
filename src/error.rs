use thiserror::Error;

/// An expected rejection, distinct from operational failures.
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
