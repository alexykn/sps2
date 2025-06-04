//! Feature management for Starlark recipes

use crate::recipe::BuildStep;
use crate::starlark::context::BuildContext;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneType;
use starlark::values::{Value, ValueLike};

/// Register feature management functions as globals
pub fn register_globals(builder: &mut GlobalsBuilder) {
    features_module(builder);
}

/// Feature management functions exposed to Starlark
#[starlark_module]
#[allow(clippy::unnecessary_wraps)]
fn features_module(builder: &mut GlobalsBuilder) {
    /// Enable a feature flag
    ///
    /// Example:
    /// - enable_feature(ctx, "ssl")
    /// - enable_feature(ctx, "opengl")
    fn enable_feature<'v>(ctx: Value<'v>, name: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx
            .features
            .borrow_mut()
            .insert(name.to_string(), true);
        build_ctx.add_step(BuildStep::EnableFeature {
            name: name.to_string(),
        });
        Ok(NoneType)
    }

    /// Disable a feature flag
    ///
    /// Example:
    /// - disable_feature(ctx, "debug")
    /// - disable_feature(ctx, "tests")
    fn disable_feature<'v>(ctx: Value<'v>, name: &str) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;

        build_ctx
            .features
            .borrow_mut()
            .insert(name.to_string(), false);
        build_ctx.add_step(BuildStep::DisableFeature {
            name: name.to_string(),
        });
        Ok(NoneType)
    }

    /// Execute steps conditionally based on features
    ///
    /// Example:
    /// ```text
    /// with_features(ctx, ["ssl", "tls"], lambda: [
    ///     configure(ctx, ["--with-ssl", "--with-tls"]),
    ///     make(ctx, [])
    /// ])
    /// ```
    fn with_features<'v>(
        ctx: Value<'v>,
        features: Value<'v>,
        steps_fn: Value<'v>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        // Unpack BuildContext from the Value
        let build_ctx = ctx
            .downcast_ref::<BuildContext>()
            .ok_or_else(|| anyhow::anyhow!("First argument must be a BuildContext"))?;
        // Extract feature names from the list
        let feature_names =
            if let Some(list) = starlark::values::list::ListRef::from_value(features) {
                let mut names = Vec::new();
                for item in list.iter() {
                    if let Some(s) = item.unpack_str() {
                        names.push(s.to_string());
                    } else {
                        return Err(anyhow::anyhow!("Feature names must be strings"));
                    }
                }
                names
            } else {
                return Err(anyhow::anyhow!("Features must be a list of strings"));
            };

        // Check if all features are enabled
        let all_enabled = {
            let feature_map = build_ctx.features.borrow();
            feature_names
                .iter()
                .all(|f| feature_map.get(f).copied().unwrap_or(false))
        };

        if all_enabled {
            // Execute the callback function
            eval.eval_function(steps_fn, &[], &[])
                .map_err(|e| anyhow::anyhow!(e))?;
        }

        // Record the conditional execution
        build_ctx.add_step(BuildStep::WithFeatures {
            features: feature_names,
            steps: vec![], // Steps are executed inline
        });

        Ok(NoneType)
    }
}
