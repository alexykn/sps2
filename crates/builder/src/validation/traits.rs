//! Generic abstractions for post‑build actions.

use crate::validation::diagnostics::DiagnosticCollector;
use crate::validation::reports::Report;
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use std::future::Future;

pub trait Action: Send + Sync + 'static {
    /// Human readable label (emitted in events).
    const NAME: &'static str;

    /// Execute the action and return a [`Report`].
    /// Validators should ignore the findings parameter.
    /// Patchers may use the findings to target specific files.
    fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
        findings: Option<&DiagnosticCollector>,
    ) -> impl Future<Output = Result<Report, Error>> + Send;
}

pub trait Validator: Action {}
pub trait Patcher: Action {}
