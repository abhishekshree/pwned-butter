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

#[cfg(test)]
mod tests {
    use super::Rejection;

    #[test]
    fn rejection_reasons_have_stable_codes() {
        assert_eq!(Rejection::OutOfScopeCity.to_string(), "out_of_scope_city");
        assert_eq!(
            Rejection::NoSubstantiveDetails.to_string(),
            "no_substantive_details"
        );
    }
}
