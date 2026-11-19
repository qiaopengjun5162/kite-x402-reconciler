//! Exit-status policy for gating unattended runs on reconciliation findings.
//!
//! A reconciler usually runs without a human watching: a cron job or a CI step
//! wants the report artifact **and** a non-zero exit status when the ledger is
//! not clean. [`FailOn`] expresses that threshold once, so `validate`,
//! `duplicates` and `report` cannot drift apart in how strict they are.

use std::str::FromStr;

use crate::validate::ValidationReport;

/// Lowest finding severity that makes a run fail.
///
/// Ordering matters: `None < Error < Warning`. `validate --strict` uses
/// [`Ord::max`] to raise the configured threshold to at least [`FailOn::Error`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailOn {
    /// Never fail; findings stay informational.
    #[default]
    None,
    /// Fail when at least one error-severity finding exists.
    Error,
    /// Fail on errors or warnings.
    Warning,
}

impl FailOn {
    /// Accepted command-line values, in help order.
    pub const POSSIBLE_VALUES: [&'static str; 3] = ["none", "error", "warning"];

    /// True when `report` holds a finding at or above this threshold.
    #[must_use]
    pub fn violated(self, report: &ValidationReport) -> bool {
        match self {
            Self::None => false,
            Self::Error => report.errors > 0,
            Self::Warning => report.errors > 0 || report.warnings > 0,
        }
    }

    /// The threshold name, for the message printed when the gate trips.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

impl FromStr for FailOn {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "error" => Ok(Self::Error),
            "warning" => Ok(Self::Warning),
            other => Err(format!(
                "invalid fail-on level {other:?}; expected one of {}",
                Self::POSSIBLE_VALUES.join(", ")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{Issue, Severity};

    fn report(errors: usize, warnings: usize) -> ValidationReport {
        let mut issues = Vec::new();
        for _ in 0..errors {
            issues.push(Issue {
                index: 0,
                code: "invalid-network",
                severity: Severity::Error,
                message: "x".into(),
            });
        }
        for _ in 0..warnings {
            issues.push(Issue {
                index: 0,
                code: "zero-amount",
                severity: Severity::Warning,
                message: "x".into(),
            });
        }
        ValidationReport::from_issues(0, issues)
    }

    #[test]
    fn none_never_trips() {
        assert!(!FailOn::None.violated(&report(0, 0)));
        assert!(!FailOn::None.violated(&report(3, 4)));
    }

    #[test]
    fn error_ignores_warnings() {
        assert!(!FailOn::Error.violated(&report(0, 5)));
        assert!(FailOn::Error.violated(&report(1, 0)));
    }

    #[test]
    fn warning_trips_on_either_kind() {
        assert!(!FailOn::Warning.violated(&report(0, 0)));
        assert!(FailOn::Warning.violated(&report(0, 1)));
        assert!(FailOn::Warning.violated(&report(1, 0)));
    }

    #[test]
    fn parse_accepts_exactly_the_documented_values() {
        assert_eq!("none".parse::<FailOn>(), Ok(FailOn::None));
        assert_eq!("error".parse::<FailOn>(), Ok(FailOn::Error));
        assert_eq!("warning".parse::<FailOn>(), Ok(FailOn::Warning));

        let err = "ERROR".parse::<FailOn>().unwrap_err();
        assert!(err.contains("none, error, warning"), "{err}");
    }

    #[test]
    fn strict_can_only_raise_the_threshold() {
        // Mirrors the `validate --strict` rule: never silently relax.
        assert_eq!(FailOn::None.max(FailOn::Error), FailOn::Error);
        assert_eq!(FailOn::Warning.max(FailOn::Error), FailOn::Warning);
    }
}
