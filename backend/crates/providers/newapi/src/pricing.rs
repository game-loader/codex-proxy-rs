//! new-api 内置价目：中转上游只按 standard 档计价，管理端覆盖按档位优先

use std::collections::BTreeMap;

use gateway_core::metering::{ModelPriceOverride, TokenPrice, TokenPriceOverride};

const STANDARD_BAND: &str = "standard";
const DEFAULT_MULTIPLIER_BPS: u32 = 10_000;

/// 单价为 USD / 百万 Token，依次为输入、输出、缓存读取、缓存写入
type Rates = (&'static str, &'static str, &'static str, &'static str);

// 中转同名 GPT 模型沿用 OpenAI 官方标准档，缓存写入为输入价的 125%
// 来源：https://developers.openai.com/api/docs/pricing ，与 OpenAI Provider 内置价目同期核验
// Kimi 来源：https://platform.kimi.ai/docs/pricing/chat ，2026-11-09 核验，上下文长度不分档
// Kimi 缓存写入单价随 TTL 档位变化且中转不回报档位，按未命中输入价计
// k3-256k 为 new-api 渠道别名，按 Kimi K3 计价
const PRICING_RULES: &[(&[&str], Rates)] = &[
    (&["gpt-6.2-astra"], ("8", "40", "0.4", "10")),
    (&["gpt-6-astra"], ("10", "50", "1", "12.5")),
    (&["gpt-6.2-sol", "gpt-6.1-sol"], ("2", "10", "0.1", "2.5")),
    (&["gpt-6.1-terra"], ("1", "5", "0.05", "1.25")),
    (&["gpt-6-sol"], ("2", "10", "0.2", "2.5")),
    (&["gpt-6-luna"], ("0.1", "0.5", "0.01", "0.125")),
    (&["gpt-5.6-sol", "gpt-5.6"], ("5", "30", "0.5", "6.25")),
    (&["gpt-5.6-terra"], ("2", "12", "0.2", "2.5")),
    (&["gpt-5.6-luna"], ("0.2", "1.2", "0.02", "0.25")),
    (&["kimi-k3.1-pro", "k3.1-pro"], ("2", "10", "0.2", "2")),
    (
        &["kimi-k3.1-pro-highspeed", "k3.1-pro-highspeed"],
        ("4", "20", "0.2", "4"),
    ),
    (&["kimi-k3", "k3", "k3-256k"], ("3", "15", "0.3", "3")),
    // Z.ai 国际站：https://docs.z.ai/guides/overview/pricing ，2026-11-24 核验
    // 缓存存储限时免费，写入 Token 仍按未命中输入价计
    // GLM-5.4 暂用 5.3 的本地估价，管理员可按中转商报价覆盖
    (&["glm-5.3", "glm-5.4"], ("1.4", "4.4", "0.26", "1.4")),
    // 来源：https://api-docs.deepseek.com/quick_start/pricing/ ，2026-11-24 核验
    // 中转不提供官方折扣是否适用的依据，固定采用 USD 高峰参考价
    (
        &[
            "deepseek-flash",
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
        ],
        ("0.3", "1.2", "0.006", "0.3"),
    ),
];

fn price(value: &str) -> TokenPrice {
    TokenPrice::try_from(value.to_owned()).expect("内置价格精度合法")
}

fn builtin_rates(model: &str) -> Option<TokenPriceOverride> {
    PRICING_RULES
        .iter()
        .find(|(models, _)| models.contains(&model))
        .map(
            |(_, (input, output, cache_read, cache_write))| TokenPriceOverride {
                input: price(input),
                output: price(output),
                cache_read: price(cache_read),
                cache_write: price(cache_write),
            },
        )
}

/// 管理端展示的内置价目
pub(crate) fn pricing_catalog() -> BTreeMap<String, ModelPriceOverride> {
    PRICING_RULES
        .iter()
        .flat_map(|(models, _)| models.iter())
        .filter_map(|model| {
            let rates = builtin_rates(model)?;
            Some((
                (*model).to_owned(),
                ModelPriceOverride {
                    multiplier_bps: DEFAULT_MULTIPLIER_BPS,
                    bands: BTreeMap::from([(STANDARD_BAND.to_owned(), rates)]),
                },
            ))
        })
        .collect()
}

/// 冻结配置缺少 standard 档时回退内置单价，倍率仍以人工覆盖为准
pub(crate) fn effective_pricing(
    model: &str,
    configured: Option<&ModelPriceOverride>,
) -> Option<ModelPriceOverride> {
    let rates = configured
        .and_then(|pricing| pricing.bands.get(STANDARD_BAND).cloned())
        .or_else(|| builtin_rates(model))?;
    Some(ModelPriceOverride {
        multiplier_bps: configured.map_or(DEFAULT_MULTIPLIER_BPS, |pricing| pricing.multiplier_bps),
        bands: BTreeMap::from([(STANDARD_BAND.to_owned(), rates)]),
    })
}
