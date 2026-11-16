//! Deterministic fixture execution summaries and immutable evidence reports.

use crate::{
    canonical,
    contracts::{
        CaseObservation, CaseResult, EvidenceReport, EvidenceSchema, ExpectedStatus, FixtureCase,
        FixtureSuite, Outcome, Profile, WireDocument,
    },
    diagnostic::{Error, Result},
    digest::Digest,
};
use std::collections::BTreeSet;

/// Digests and implementation metadata that identify one fixture run.
#[derive(Debug, Clone)]
pub struct FixtureInputs {
    pub subject: Digest,
    pub kernel: Digest,
    pub fixtures: Digest,
    pub manifest: Digest,
    pub tool_version: String,
    pub environment: String,
}

/// Run each declared fixture once and record both the expectation and observed
/// result. `suite` must be a validated `FixtureSuite`. An operational outcome
/// (Denied, Failed, etc.) remains an observation; setup/evaluation errors are
/// retained as diagnostics on failed cases.
pub fn run_fixtures<F>(
    inputs: FixtureInputs,
    suite: &FixtureSuite,
    mut execute: F,
) -> EvidenceReport
where
    F: FnMut(&FixtureCase) -> Result<CaseObservation>,
{
    let mut allowed = BTreeSet::new();
    let cases = suite
        .cases
        .iter()
        .map(|fixture| {
            allowed.extend(fixture.allow.iter().cloned());
            match execute(fixture) {
                Ok(actual) => {
                    let passed = matches_expectation(fixture, &actual);
                    CaseResult {
                        name: fixture.name.clone(),
                        passed,
                        detail: describe_result(fixture, &actual, passed),
                        expected: Some(fixture.expect.clone()),
                        actual: Some(actual),
                        diagnostic: None,
                        allow: fixture.allow.clone(),
                    }
                }
                Err(error) => CaseResult {
                    name: fixture.name.clone(),
                    passed: false,
                    detail: format!(
                        "fixture execution stopped before an outcome: {}",
                        error.diagnostic
                    ),
                    expected: Some(fixture.expect.clone()),
                    actual: None,
                    diagnostic: Some(error.diagnostic),
                    allow: fixture.allow.clone(),
                },
            }
        })
        .collect();

    EvidenceReport {
        schema: EvidenceSchema::V03,
        profile: Profile::Slice0,
        subject: inputs.subject,
        tool_version: inputs.tool_version,
        kernel: inputs.kernel,
        fixtures: inputs.fixtures,
        manifest: inputs.manifest,
        environment: inputs.environment,
        allow: allowed.into_iter().collect(),
        cases,
        limitations: vec![
            "Evidence applies only to the named fixtures, subject, manifest, kernel, and slice0 profile.".into(),
        ],
    }
}

/// Return the content address for an immutable report. The digest is external
/// to the report body, avoiding a self-referential identity field.
pub fn report_digest(report: &EvidenceReport) -> Result<Digest> {
    let mut normalized = report.clone();
    normalized.normalize()?;
    let json = serde_json::to_value(normalized)
        .map_err(|error| Error::new("E_EVIDENCE", error.to_string()))?;
    canonical::digest(&json)
}

fn matches_expectation(fixture: &FixtureCase, actual: &CaseObservation) -> bool {
    let expected = &fixture.expect;
    stdout_matches(&expected.stdout, &actual.stdout)
        && status_matches(expected.status, &actual.outcome)
        && expected.exit_code == actual.exit_code
        && actual.outcome.exit_code() as u8 == actual.exit_code
        && result_matches(expected.status, &expected.result, &actual.outcome)
}

fn stdout_matches(expected: &str, actual: &[u8]) -> bool {
    expected.as_bytes() == actual
}

fn status_matches(expected: ExpectedStatus, actual: &Outcome) -> bool {
    matches!(
        (expected, actual),
        (ExpectedStatus::Succeeded, Outcome::Succeeded { .. })
            | (ExpectedStatus::Skipped, Outcome::Skipped { .. })
            | (ExpectedStatus::Denied, Outcome::Denied { .. })
            | (
                ExpectedStatus::AwaitingApproval,
                Outcome::AwaitingApproval { .. }
            )
            | (ExpectedStatus::Failed, Outcome::Failed { .. })
            | (ExpectedStatus::Cancelled, Outcome::Cancelled { .. })
    )
}

fn result_matches(
    expected_status: ExpectedStatus,
    expected: &crate::value::Value,
    actual: &Outcome,
) -> bool {
    match (expected_status, actual) {
        (ExpectedStatus::Succeeded, Outcome::Succeeded { value }) => expected == value,
        // Non-success outcomes have no result value in the receipt contract.
        (ExpectedStatus::Skipped, Outcome::Skipped { .. })
        | (ExpectedStatus::Denied, Outcome::Denied { .. })
        | (ExpectedStatus::AwaitingApproval, Outcome::AwaitingApproval { .. })
        | (ExpectedStatus::Failed, Outcome::Failed { .. })
        | (ExpectedStatus::Cancelled, Outcome::Cancelled { .. }) => true,
        _ => false,
    }
}

fn describe_result(fixture: &FixtureCase, actual: &CaseObservation, passed: bool) -> String {
    if passed {
        return "matched expected stdout, result, status, and exit code".into();
    }

    let mut mismatches = Vec::new();
    if !stdout_matches(&fixture.expect.stdout, &actual.stdout) {
        mismatches.push(format!(
            "stdout expected {:?}, got {:?}",
            fixture.expect.stdout,
            String::from_utf8_lossy(&actual.stdout)
        ));
    }
    if !status_matches(fixture.expect.status, &actual.outcome) {
        mismatches.push(format!(
            "status expected {:?}, got {:?}",
            fixture.expect.status, actual.outcome
        ));
    }
    if fixture.expect.exit_code != actual.exit_code
        || actual.outcome.exit_code() as u8 != actual.exit_code
    {
        mismatches.push(format!(
            "exit code expected {}, got {} (outcome maps to {})",
            fixture.expect.exit_code,
            actual.exit_code,
            actual.outcome.exit_code() as u8
        ));
    }
    if !result_matches(
        fixture.expect.status,
        &fixture.expect.result,
        &actual.outcome,
    ) {
        mismatches.push(format!(
            "result expected {:?}, got {:?}",
            fixture.expect.result, actual.outcome
        ));
    }
    if mismatches.is_empty() {
        "execution did not match the fixture expectation".into()
    } else {
        mismatches.join("; ")
    }
}
