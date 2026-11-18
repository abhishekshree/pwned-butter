use strum::Display;

/// An expected business rejection, not an operational failure.
///
/// Fieldless so every failure reason is a typed variant rather than a string
/// in disguise. `Display` is derived with strum, matching `ActionType` and
/// `OutletType` in `models.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum Rejection {
    OutOfScopeCity,
    NoEstablishment,
    NoActionDate,
    QuoteLedEstablishment,
    PendingAction,
    ProductNotOutlet,
    RegulatorNotOutlet,
    HeadlineFragment,
    NoSubstantiveDetails,
}
