//! What models cost, per token, as each provider publishes it. OpenAI and Anthropic have no price
//! API, and their Models APIs carry no price, so their adapters read the Markdown their pricing
//! pages serve; OpenRouter's listing has prices of its own.
use crate::Error;
use reqwest::{Client, Url};
use std::collections::HashSet;

/// What a model costs per token, under the id the provider itself uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Price {
    pub model: String,
    pub input_per_token: f64,
    pub cached_per_token: f64,
    pub output_per_token: f64,
}

/// Which headers hold which rate in a provider's tables; a table without them is skipped.
pub(crate) struct Columns {
    pub input: &'static [&'static str],
    pub cached: &'static [&'static str],
    pub output: &'static [&'static str],
}

/// The page's Markdown. A page that yields no prices is an error, so a redesign keeps the last
/// good prices instead of erasing them.
pub(crate) async fn page(http: &Client, url: Url) -> Result<String, Error> {
    let response = crate::send(http.get(url)).await?;
    let bytes = crate::body(response).await?;
    String::from_utf8(bytes).map_err(|error| Error::InvalidResponse(Some(error.into())))
}

/// Every price in the tables under `heading`, in every section that has it. A model listed twice
/// keeps its first row, which is its lowest context tier.
/// shortcut: long-context rates are ignored, so a very long prompt reads as cheaper than it was;
/// store the tiers if that skews estimates.
pub(crate) fn from_markdown(
    markdown: &str,
    heading: &str,
    columns: &Columns,
    id: fn(&str) -> String,
) -> Result<Vec<Price>, Error> {
    let mut seen = HashSet::new();
    let prices: Vec<Price> = tables(markdown, heading)
        .iter()
        .flat_map(|table| rows(table, columns, id))
        .filter(|price| seen.insert(price.model.clone()))
        .collect();
    if prices.is_empty() {
        return Err(Error::InvalidResponse(None));
    }
    Ok(prices)
}

fn level(line: &str) -> usize {
    line.chars().take_while(|c| *c == '#').count()
}

/// Runs of table lines under `heading`, up to the next heading at its level or above.
fn tables<'a>(markdown: &'a str, heading: &str) -> Vec<Vec<&'a str>> {
    let mut inside = false;
    let mut tables: Vec<Vec<&str>> = vec![Vec::new()];
    for line in markdown.lines().map(str::trim) {
        if line.starts_with('#') {
            inside = line == heading || (inside && level(line) > level(heading));
        }
        if inside && line.starts_with('|') {
            tables.last_mut().into_iter().for_each(|t| t.push(line));
        } else if tables.last().is_some_and(|t| !t.is_empty()) {
            tables.push(Vec::new());
        }
    }
    tables
}

fn cells(line: &str) -> Vec<&str> {
    line.trim_matches('|').split('|').map(str::trim).collect()
}

fn rows(table: &[&str], columns: &Columns, id: fn(&str) -> String) -> Vec<Price> {
    let Some(header) = table.first().map(|line| cells(line)) else {
        return Vec::new();
    };
    let column = |names: &[&str]| header.iter().position(|cell| names.contains(cell));
    let (Some(input), Some(cached), Some(output)) = (
        column(columns.input),
        column(columns.cached),
        column(columns.output),
    ) else {
        return Vec::new();
    };
    let price = |line: &&str| {
        let row = cells(line);
        let input_rate = rate(row.get(input)?)?;
        Some(Price {
            model: id(name(row.first()?)),
            input_per_token: input_rate,
            // A model with no cached rate (shown as `-`) bills cached input as input.
            cached_per_token: row.get(cached).and_then(|c| rate(c)).unwrap_or(input_rate),
            output_per_token: rate(row.get(output)?)?,
        })
    };
    table[1..].iter().filter_map(price).collect()
}

/// `Claude Opus 4.1 ([retired...](...))` or `gpt-5.5 (<272K context length)`, without the note.
fn name(cell: &str) -> &str {
    let end = cell.find([' ', '<']).filter(|_| !cell.contains(' '));
    let note = cell.find(" (").or(end).unwrap_or(cell.len());
    cell[..note].trim()
}

/// `$0.25 / MTok<sup>1</sup>` or `$2.00`, per million tokens, as a rate per token.
fn rate(cell: &str) -> Option<f64> {
    let number = cell.strip_prefix('$')?;
    let end = number
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(number.len());
    let per_million: f64 = number[..end].parse().ok()?;
    // Nothing costs this much per million tokens; a larger number is a misread cell.
    (per_million <= 10_000.0).then_some(per_million / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: Columns = Columns {
        input: &["In"],
        cached: &["Hit"],
        output: &["Out"],
    };

    #[test]
    fn rates_read_as_per_token_whatever_follows_them() {
        assert_eq!(rate("$2.00"), Some(2e-6));
        assert_eq!(rate("$0.25 / MTok<sup>1</sup>"), Some(0.25e-6));
        assert_eq!(rate("-"), None);
        assert_eq!(rate("---"), None);
        assert_eq!(rate("$99999"), None);
    }

    #[test]
    fn notes_leave_the_name() {
        assert_eq!(name("gpt-5.5 (<272K context length)"), "gpt-5.5");
        assert_eq!(
            name("Claude Opus 4.1 ([retired](https://x))"),
            "Claude Opus 4.1"
        );
        assert_eq!(name("Claude Opus 5.5"), "Claude Opus 5.5");
    }

    #[test]
    fn only_the_named_sections_and_recognized_tables_count() {
        let page = "## Other\n| Model | In | Out | Hit |\n| m0 | $9 | $9 | $9 |\n\
            ## Wanted\n#### Sub\n| Model | In | Out | Hit |\n| --- | --- | --- | --- |\n\
            | m1 | $1 | $2 | - |\n| m1 (long) | $5 | $6 | $7 |\n\n\
            | Unrelated | Size |\n| m2 | $1 |\n\n\
            | Model | Hit | In | Out |\n| m3 | $0.5 | $1 | $2 |\n## Later\n| Model | In | Out | Hit |\n\
            | m4 | $9 | $9 | $9 |\n";
        let prices = from_markdown(page, "## Wanted", &COLUMNS, str::to_owned).unwrap();
        let names: Vec<&str> = prices.iter().map(|p| p.model.as_str()).collect();
        assert_eq!(names, ["m1", "m3"]);
        assert_eq!(prices[0].cached_per_token, prices[0].input_per_token);
        assert_eq!(prices[1].cached_per_token, 0.5e-6);
    }

    #[test]
    fn a_page_without_prices_is_an_error_so_the_last_good_ones_stay() {
        let redesigned = "# Pricing\nSee our new calculator.\n";
        assert!(from_markdown(redesigned, "## Wanted", &COLUMNS, str::to_owned).is_err());
    }
}
