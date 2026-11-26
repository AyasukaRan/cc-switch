//! Cost Calculator - 计算 API 请求成本
//!
//! 使用高精度 Decimal 类型避免浮点数精度问题

use super::parser::TokenUsage;
use rust_decimal::Decimal;
use std::str::FromStr;

/// 成本明细
#[derive(Debug, Clone)]
pub struct CostBreakdown {
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub cache_read_cost: Decimal,
    pub cache_creation_cost: Decimal,
    pub total_cost: Decimal,
}

/// 模型定价信息
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPricing {
    pub input_cost_per_million: Decimal,
    pub output_cost_per_million: Decimal,
    pub cache_read_cost_per_million: Decimal,
    pub cache_creation_cost_per_million: Decimal,
}

pub(crate) fn normalize_codex_service_tier(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "default" | "standard" => Some("default"),
        "fast" | "priority" => Some("priority"),
        "ultrafast" => Some("ultrafast"),
        "flex" => Some("flex"),
        _ => None,
    }
}

// Ratios to the configured standard token prices. Only apply a tier published
// for this model: https://developers.openai.com/api/docs/pricing
fn codex_tier_ratio(model: &str, tier: &str, created_at: i64) -> Option<(u32, u32)> {
    match (tier, model) {
        ("ultrafast", "gpt-6.2-astra" | "gpt-6.2-sol") => {
            // The introductory 3× rate ends 2027-03-01 00:00 Pacific (08:00 UTC).
            Some((if created_at < 1_803_888_000 { 3 } else { 6 }, 1))
        }
        ("ultrafast", "gpt-6-astra" | "gpt-6.1-sol") => Some((6, 1)),
        ("priority", "gpt-5.5") => Some((5, 2)),
        ("priority", "gpt-5-mini") => Some((9, 5)),
        ("priority", "gpt-4.1" | "gpt-4.1-mini" | "o3") => Some((7, 4)),
        ("priority", "gpt-4o") => Some((17, 10)),
        ("priority", "gpt-4o-mini") => Some((5, 3)),
        ("priority", "gpt-6.2-astra" | "gpt-6.2-sol" | "gpt-5.3-codex") => Some((2, 1)),
        (
            "flex",
            "gpt-5.5" | "gpt-5.5-pro" | "gpt-5.4-pro" | "gpt-5.4-nano" | "gpt-5-mini"
            | "gpt-5-nano" | "o3",
        ) => Some((1, 2)),
        (
            tier,
            "gpt-6-astra" | "gpt-6.1-sol" | "gpt-6.1-terra" | "gpt-6-sol" | "gpt-6-luna"
            | "gpt-5.6" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" | "gpt-5.4"
            | "gpt-5.4-mini" | "gpt-5.2" | "gpt-5.1" | "gpt-5",
        ) => match tier {
            "priority" => Some((2, 1)),
            "flex" => Some((1, 2)),
            _ => None,
        },
        _ => None,
    }
}

/// 成本计算器
pub struct CostCalculator;

impl CostCalculator {
    /// 按 app_type 选择输入 token 语义后计算成本。
    ///
    /// Codex/OpenAI Responses 与 Gemini 的输入 token 字段包含 cache read 部分；
    /// Claude/Anthropic 的 input_tokens 已经是 fresh input。
    pub fn calculate_for_app(
        app_type: &str,
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
    ) -> CostBreakdown {
        let input_includes_cache_read =
            crate::services::sql_helpers::is_cache_inclusive_app(app_type);
        Self::calculate_with_cache_semantics(
            usage,
            pricing,
            cost_multiplier,
            input_includes_cache_read,
        )
    }

    fn calculate_with_cache_semantics(
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
        input_includes_cache_read: bool,
    ) -> CostBreakdown {
        let million = Decimal::from(1_000_000);

        // OpenAI/Gemini 风格的 input_tokens 包含缓存读取和写入，需要扣除后再按输入价计费；
        // Claude/Anthropic 风格的 input_tokens 已经是 fresh input，不能再次扣减。
        let billable_input_tokens = if input_includes_cache_read {
            usage
                .input_tokens
                .saturating_sub(usage.cache_read_tokens)
                .saturating_sub(usage.cache_creation_tokens)
        } else {
            usage.input_tokens
        };

        // 各项基础成本（不含倍率）
        let input_cost =
            Decimal::from(billable_input_tokens) * pricing.input_cost_per_million / million;
        let output_cost =
            Decimal::from(usage.output_tokens) * pricing.output_cost_per_million / million;
        let cache_read_cost =
            Decimal::from(usage.cache_read_tokens) * pricing.cache_read_cost_per_million / million;
        // Anthropic 5 分钟缓存写入为输入价 1.25 倍、1 小时写入为 2 倍；定价表的
        // cache_creation 单价对应 5 分钟写入，1 小时写入按其 1.6 倍计。
        let cache_creation_1h_tokens = usage
            .cache_creation_1h_tokens
            .min(usage.cache_creation_tokens);
        let cache_creation_cost =
            (Decimal::from(usage.cache_creation_tokens - cache_creation_1h_tokens)
                + Decimal::from(cache_creation_1h_tokens) * Decimal::new(16, 1))
                * pricing.cache_creation_cost_per_million
                / million;

        // 总成本 = 各项基础成本之和 × 倍率
        let base_total = input_cost + output_cost + cache_read_cost + cache_creation_cost;
        let total_cost = base_total * cost_multiplier;

        CostBreakdown {
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost,
            total_cost,
        }
    }

    pub fn try_calculate_for_app(
        app_type: &str,
        usage: &TokenUsage,
        pricing: Option<&ModelPricing>,
        cost_multiplier: Decimal,
    ) -> Option<CostBreakdown> {
        pricing.map(|p| Self::calculate_for_app(app_type, usage, p, cost_multiplier))
    }
}

impl ModelPricing {
    /// Apply the recorded service tier to the token prices, independently of
    /// any provider cost multiplier. Missing and unpriced tiers use the base rate.
    pub(crate) fn for_codex_request(
        &self,
        model: &str,
        service_tier: Option<&str>,
        created_at: i64,
    ) -> Self {
        let Some(tier) = service_tier.and_then(normalize_codex_service_tier) else {
            return self.clone();
        };
        if tier == "default" {
            return self.clone();
        }
        let candidates = crate::services::usage_stats::model_pricing_candidates(model);
        let Some((id, (numerator, denominator))) = candidates
            .iter()
            .map(|candidate| candidate.model_id.as_str())
            .find_map(|id| codex_tier_ratio(id, tier, created_at).map(|ratio| (id, ratio)))
        else {
            return self.clone();
        };
        let adjust = |rate: Decimal| rate * Decimal::from(numerator) / Decimal::from(denominator);
        // GPT-5.4 Flex lists $0.13 per million cache reads against a $0.25 base.
        let cache_read = if id == "gpt-5.4" && tier == "flex" {
            self.cache_read_cost_per_million * Decimal::new(52, 2)
        } else {
            adjust(self.cache_read_cost_per_million)
        };
        Self {
            input_cost_per_million: adjust(self.input_cost_per_million),
            output_cost_per_million: adjust(self.output_cost_per_million),
            cache_read_cost_per_million: cache_read,
            cache_creation_cost_per_million: adjust(self.cache_creation_cost_per_million),
        }
    }

    /// 从字符串创建定价信息
    pub fn from_strings(
        input: &str,
        output: &str,
        cache_read: &str,
        cache_creation: &str,
    ) -> Result<Self, rust_decimal::Error> {
        Ok(Self {
            input_cost_per_million: Decimal::from_str(input)?,
            output_cost_per_million: Decimal::from_str(output)?,
            cache_read_cost_per_million: Decimal::from_str(cache_read)?,
            cache_creation_cost_per_million: Decimal::from_str(cache_creation)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_service_prices_follow_the_model_and_request_date() {
        let prices = ModelPricing::from_strings("8", "40", "0.4", "10").unwrap();
        for (model, tier, time, factor) in [
            ("gpt-6.2-astra", "ultrafast", 1_803_887_999, 3),
            (
                "openai/gpt-6.2-astra-2026-11-17",
                "ultrafast",
                1_803_888_000,
                6,
            ),
            ("gpt-6-astra@high", "ultrafast", 1_800_000_000, 6),
            ("gpt-6.2-sol", "fast", 1_800_000_000, 2),
            ("gpt-6.2-sol", "standard", 1_800_000_000, 1),
            ("custom-relay", "ultrafast", 1_800_000_000, 1),
            ("gpt-6.1-terra", "ultrafast", 1_800_000_000, 1),
        ] {
            let actual = prices.for_codex_request(model, Some(tier), time);
            let factor = Decimal::from(factor);
            assert_eq!(
                actual.input_cost_per_million,
                prices.input_cost_per_million * factor
            );
            assert_eq!(
                actual.output_cost_per_million,
                prices.output_cost_per_million * factor
            );
            assert_eq!(
                actual.cache_read_cost_per_million,
                prices.cache_read_cost_per_million * factor
            );
            assert_eq!(
                actual.cache_creation_cost_per_million,
                prices.cache_creation_cost_per_million * factor
            );
        }
        assert_eq!(
            prices
                .for_codex_request("gpt-6-astra", Some("flex"), 0)
                .input_cost_per_million,
            Decimal::from(4)
        );
        assert_eq!(
            ModelPricing::from_strings("0.15", "0.6", "0.075", "0")
                .unwrap()
                .for_codex_request("gpt-4o-mini", Some("priority"), 0)
                .cache_read_cost_per_million,
            Decimal::new(125, 3)
        );
        assert_eq!(
            ModelPricing::from_strings("2.50", "15", "0.25", "0")
                .unwrap()
                .for_codex_request("gpt-5.4-2026-03-05", Some("flex"), 0)
                .cache_read_cost_per_million,
            Decimal::new(13, 2)
        );
    }

    #[test]
    fn test_cost_calculation() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 100,
            cache_creation_1h_tokens: 0,
            model: None,
            message_id: None,
        };

        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        let multiplier = Decimal::from_str("1.0").unwrap();

        let cost = CostCalculator::calculate_for_app("claude", &usage, &pricing, multiplier);

        // Claude/Anthropic 语义：input_tokens 已经不含 cache_read_tokens
        // input: 1000 * 3.0 / 1M = 0.003
        assert_eq!(cost.input_cost, Decimal::from_str("0.003").unwrap());
        // output: 500 * 15.0 / 1M = 0.0075
        assert_eq!(cost.output_cost, Decimal::from_str("0.0075").unwrap());
        // cache_read: 200 * 0.3 / 1M = 0.00006
        assert_eq!(cost.cache_read_cost, Decimal::from_str("0.00006").unwrap());
        // cache_creation: 100 * 3.75 / 1M = 0.000375
        assert_eq!(
            cost.cache_creation_cost,
            Decimal::from_str("0.000375").unwrap()
        );
        // total: 0.003 + 0.0075 + 0.00006 + 0.000375 = 0.010935
        assert_eq!(cost.total_cost, Decimal::from_str("0.010935").unwrap());
    }

    #[test]
    fn test_claude_1h_cache_writes_billed_at_1h_rate() {
        // Claude Opus 4.8：5m 缓存写入 $6.25/M，1h 缓存写入 $10/M（输入价的 2 倍）
        let pricing = ModelPricing::from_strings("5", "25", "0.50", "6.25").unwrap();
        let usage = serde_json::json!({
            "input_tokens": 2,
            "output_tokens": 1,
            "cache_read_input_tokens": 0,
            "cache_creation_input_tokens": 1000,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 400,
                "ephemeral_1h_input_tokens": 600
            }
        });
        let response = serde_json::json!({ "model": "claude-opus-4-8", "usage": usage });
        let events = [
            serde_json::json!({
                "type": "message_start",
                "message": { "model": "claude-opus-4-8", "usage": usage }
            }),
            serde_json::json!({ "type": "message_delta", "usage": { "output_tokens": 5 } }),
        ];

        for parsed in [
            TokenUsage::from_claude_response(&response),
            TokenUsage::from_claude_stream_events(&events),
        ] {
            let cost = CostCalculator::calculate_for_app(
                "claude",
                &parsed.unwrap(),
                &pricing,
                Decimal::ONE,
            );
            // 400 × 6.25 / 1M + 600 × 10 / 1M = 0.0085
            assert_eq!(
                cost.cache_creation_cost,
                Decimal::from_str("0.0085").unwrap()
            );
        }
    }

    #[test]
    fn test_cost_calculation_for_cache_inclusive_app() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 100,
            cache_creation_1h_tokens: 0,
            model: None,
            message_id: None,
        };

        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        let multiplier = Decimal::from_str("1.0").unwrap();

        let cost = CostCalculator::calculate_for_app("codex", &usage, &pricing, multiplier);

        // Codex/OpenAI 语义：input_tokens 包含 cache read/write，两桶都需扣除。
        assert_eq!(cost.input_cost, Decimal::from_str("0.0021").unwrap());
        assert_eq!(cost.output_cost, Decimal::from_str("0.0075").unwrap());
        assert_eq!(cost.cache_read_cost, Decimal::from_str("0.00006").unwrap());
        assert_eq!(
            cost.cache_creation_cost,
            Decimal::from_str("0.000375").unwrap()
        );
        assert_eq!(cost.total_cost, Decimal::from_str("0.010035").unwrap());
    }

    #[test]
    fn grokbuild_does_not_double_bill_cached_input() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 0,
            cache_read_tokens: 600,
            cache_creation_tokens: 0,
            cache_creation_1h_tokens: 0,
            model: None,
            message_id: None,
        };
        let pricing = ModelPricing::from_strings("10", "0", "1", "0").unwrap();

        let cost = CostCalculator::calculate_for_app("grokbuild", &usage, &pricing, Decimal::ONE);

        assert_eq!(cost.input_cost, Decimal::from_str("0.004").unwrap());
        assert_eq!(cost.cache_read_cost, Decimal::from_str("0.0006").unwrap());
        assert_eq!(cost.total_cost, Decimal::from_str("0.0046").unwrap());
    }

    #[test]
    fn test_cost_multiplier() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cache_creation_1h_tokens: 0,
            model: None,
            message_id: None,
        };

        let pricing = ModelPricing::from_strings("3.0", "15.0", "0", "0").unwrap();
        let multiplier = Decimal::from_str("1.5").unwrap();

        let cost = CostCalculator::calculate_for_app("claude", &usage, &pricing, multiplier);

        // input_cost: 基础价格（不含倍率）= 1000 * 3.0 / 1M = 0.003
        assert_eq!(cost.input_cost, Decimal::from_str("0.003").unwrap());
        // total_cost: 基础价格 × 倍率 = 0.003 * 1.5 = 0.0045
        assert_eq!(cost.total_cost, Decimal::from_str("0.0045").unwrap());
    }

    #[test]
    fn test_decimal_precision() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 1,
            cache_creation_tokens: 1,
            cache_creation_1h_tokens: 0,
            model: None,
            message_id: None,
        };

        let pricing = ModelPricing::from_strings("0.075", "0.3", "0.01875", "0.075").unwrap();
        let multiplier = Decimal::from_str("1.0").unwrap();

        let cost = CostCalculator::calculate_for_app("claude", &usage, &pricing, multiplier);

        // 验证高精度计算
        assert!(cost.total_cost > Decimal::ZERO);
        assert!(cost.total_cost.to_string().len() > 2); // 确保保留了小数位
    }
}
