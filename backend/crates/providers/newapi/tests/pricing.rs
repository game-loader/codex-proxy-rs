//! 内置价目：未配置的中转模型按内置单价计费，管理端覆盖按档位与倍率优先

use std::collections::BTreeMap;

use gateway_admin::ports::provider::{ProviderAdmin, ProviderUsagePricingInput};
use gateway_core::metering::{ModelPriceOverride, TokenPrice, TokenPriceOverride, Usage};

use crate::support::ports_with_account;

fn admin() -> std::sync::Arc<dyn ProviderAdmin> {
    let ports = ports_with_account("acct_relay", "http://127.0.0.1:9/v1");
    provider_newapi::initialize(&ports)
        .unwrap()
        .admin_provider()
}

fn input(model: &str) -> ProviderUsagePricingInput {
    let mut usage = Usage::new();
    usage.input_tokens = Some(1_000_000);
    usage.cached_input_tokens = Some(900_000);
    usage.output_tokens = Some(100_000);
    ProviderUsagePricingInput {
        upstream_model_id: model.to_owned(),
        service_tier: None,
        usage,
    }
}

fn total(
    admin: &dyn ProviderAdmin,
    model: &str,
    pricing: Option<&ModelPriceOverride>,
) -> Option<String> {
    admin
        .evaluate_usage_pricing(&input(model), pricing)
        .unwrap()
        .map(|breakdown| breakdown.calculated_cost().total().amount().canonical())
}

fn price(value: &str) -> TokenPrice {
    TokenPrice::try_from(value.to_owned()).unwrap()
}

#[test]
fn catalog_lists_builtin_standard_prices() {
    let catalog = admin().pricing_catalog();
    let kimi = &catalog["k3.1-pro-highspeed"];
    assert_eq!(kimi.multiplier_bps, 10_000);
    assert_eq!(kimi.bands.keys().collect::<Vec<_>>(), ["standard"]);
    assert_eq!(kimi.bands["standard"].input, price("4"));
    assert_eq!(
        catalog["gpt-6.1-sol"].bands["standard"].cache_read,
        price("0.1")
    );
}

#[test]
fn unconfigured_models_use_builtin_prices() {
    let admin = admin();
    // 0.1M 未命中 × 4 + 0.9M 命中 × 0.2 + 0.1M 输出 × 20
    assert_eq!(
        total(admin.as_ref(), "k3.1-pro-highspeed", None).as_deref(),
        Some("2.58")
    );
    assert_eq!(total(admin.as_ref(), "unknown-model", None), None);
}

#[test]
fn overrides_take_precedence_by_band_and_multiplier() {
    let admin = admin();
    let multiplier_only = ModelPriceOverride {
        multiplier_bps: 20_000,
        bands: BTreeMap::new(),
    };
    assert_eq!(
        total(admin.as_ref(), "k3.1-pro-highspeed", Some(&multiplier_only)).as_deref(),
        Some("5.16")
    );
    let custom = ModelPriceOverride {
        multiplier_bps: 10_000,
        bands: BTreeMap::from([(
            "standard".to_owned(),
            TokenPriceOverride {
                input: price("1"),
                output: price("1"),
                cache_read: price("0"),
                cache_write: price("1"),
            },
        )]),
    };
    // 0.1M × 1 + 0.1M × 1
    assert_eq!(
        total(admin.as_ref(), "k3.1-pro-highspeed", Some(&custom)).as_deref(),
        Some("0.2")
    );
}
