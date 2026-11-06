//! MiniMax through its Anthropic-compatible API, which streams and replays thinking blocks like
//! Anthropic's: https://platform.minimax.io/docs/api-reference/text-anthropic-api.md
//! A pay-as-you-go key and an M Plan key reach the same endpoint; the key decides the billing.
use crate::{Anthropic, Error, anthropic::Dialect};
use reqwest::Url;

/// Both text models have a 1M window; 128K is the documented recommended output limit.
const MINIMAX: Dialect = Dialect {
    provider: "minimax",
    native: false,
    limits: (1_000_000, 131_072),
    prices,
};

const PRICES: crate::prices::Columns = crate::prices::Columns {
    input: &["Input $ / M tokens"],
    cached: &["Prompt caching Read $ / M tokens"],
    output: &["Output $ / M tokens"],
};

impl Anthropic {
    pub fn minimax() -> Result<Self, Error> {
        Self::region("https://api.minimax.io/")
    }

    /// The mainland China platform, whose keys work only there.
    pub fn minimax_cn() -> Result<Self, Error> {
        Self::region("https://api.minimax.cn/")
    }

    pub fn minimax_for_loopback_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self::for_loopback_tests(origin)?.with(MINIMAX))
    }

    fn region(origin: &str) -> Result<Self, Error> {
        let parse =
            |url: &str| Url::parse(url).map_err(|error| Error::Configuration(Some(error.into())));
        let base = parse(origin)?
            .join("anthropic/v1/")
            .map_err(|error| Error::Configuration(Some(error.into())))?;
        let pricing = parse("https://platform.minimax.io/docs/pricing/overview.md")?;
        Ok(Self::base(base, pricing)?.with(MINIMAX))
    }
}

/// The page's Markdown quotes its tabbed tables, escapes `$` and strikes through old prices.
fn prices(page: &str) -> Result<Vec<crate::Price>, Error> {
    let plain: String = page.lines().map(|line| unmarked(line) + "\n").collect();
    crate::prices::from_markdown(&plain, "## LLM", &PRICES, |name| {
        // `MiniMax-M3 ≤ 512k input tokens` is the model's first context tier.
        name.split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned()
    })
}

fn unmarked(line: &str) -> String {
    let mut line = line
        .trim_start_matches(['>', ' '])
        .replace("**", "")
        .replace("\\$", "$")
        .replace("\\*", "*");
    while let Some(start) = line.find("~~") {
        let Some(end) = line[start + 2..].find("~~") else {
            break;
        };
        line.replace_range(start..start + end + 4, "");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "## LLM\n\n\
        | **Model** | **Input** \\$ / M tokens | **Output** \\$ / M tokens | **Prompt caching Read** \\$ / M tokens |\n\
        | :- | :-: | :-: | :-: |\n\
        | **MiniMax-M3.1-Flash** | \\$0.15 | \\$0.60 | \\$0.015 |\n\n\
        > [tab] Standard\n>\n\
        > | **Model** | **Input** \\$ / M tokens | **Output** \\$ / M tokens | **Prompt caching Read** \\$ / M tokens |\n\
        > | :- | :-: | :-: | :-: |\n\
        > | **MiniMax-M3** ≤ 512k input tokens Permanent 50% off | ~~\\$0.60~~ \\$0.30 | ~~\\$2.40~~ \\$1.20 | ~~\\$0.12~~ \\$0.06 |\n\
        > | **MiniMax-M3** > 512k input tokens\\* Permanent 50% off | ~~\\$1.20~~ \\$0.60 | ~~\\$4.80~~ \\$2.40 | ~~\\$0.24~~ \\$0.12 |\n\n\
        > [tab] Priority\\*\n>\n\
        > | **Model** | **Input** \\$ / M tokens | **Output** \\$ / M tokens | **Prompt caching Read** \\$ / M tokens |\n\
        > | **MiniMax-M3** ≤ 512k input tokens Permanent 50% off | ~~\\$0.90~~ \\$0.45 | ~~\\$3.60~~ \\$1.80 | ~~\\$0.18~~ \\$0.09 |\n\n\
        ## Video\n| Model | Input $ / M tokens | Output $ / M tokens | Prompt caching Read $ / M tokens |\n\
        | MiniMax-H3 | $9 | $9 | $9 |\n";

    #[test]
    fn the_standard_rates_of_each_text_model_are_read() {
        let prices = prices(PAGE).unwrap();
        let rates: Vec<(&str, f64, f64, f64)> = prices
            .iter()
            .map(|p| {
                (
                    p.model.as_str(),
                    p.input_per_token * 1e6,
                    p.cached_per_token * 1e6,
                    p.output_per_token * 1e6,
                )
            })
            .collect();
        assert_eq!(rates.len(), 2);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let (model, input, cached, output) = rates[0];
        assert!(model == "MiniMax-M3.1-Flash" && close(input, 0.15));
        assert!(close(cached, 0.015) && close(output, 0.60));
        let (model, input, cached, output) = rates[1];
        assert!(model == "MiniMax-M3" && close(input, 0.30));
        assert!(close(cached, 0.06) && close(output, 1.20));
    }

    #[tokio::test]
    #[ignore = "reads the live pricing page"]
    async fn live_page() {
        let prices = Anthropic::minimax().unwrap().prices().await.unwrap();
        assert!(
            prices
                .iter()
                .any(|price| price.model == "MiniMax-M3.1-Flash")
        );
    }

    #[test]
    fn the_region_picks_the_platform() {
        let global = Anthropic::minimax().unwrap();
        assert_eq!(global.base.as_str(), "https://api.minimax.io/anthropic/v1/");
        let mainland = Anthropic::minimax_cn().unwrap();
        assert_eq!(
            mainland.base.as_str(),
            "https://api.minimax.cn/anthropic/v1/"
        );
    }
}
