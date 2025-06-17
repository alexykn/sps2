//! Generic abstractions for post‑build actions.

use crate::post_validation::reports::Report;
use crate::{BuildContext, BuildEnvironment};
use sps2_errors::Error;
use std::future::Future;

pub trait Action: Send + Sync + 'static {
    /// Human readable label (emitted in events).
    const NAME: &'static str;

    /// Execute the action and return a [`Report`].
    fn run(
        ctx: &BuildContext,
        env: &BuildEnvironment,
    ) -> impl Future<Output = Result<Report, Error>> + Send;
}

pub trait Validator: Action {}
pub trait Patcher: Action {}
