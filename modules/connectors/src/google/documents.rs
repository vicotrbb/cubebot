//! Google Sheets (https://developers.google.com/sheets/api/reference/rest) and Google Docs
//! (https://developers.google.com/docs/api/reference/rest).
use super::{LONGEST_TEXT, at, clip, id};
use crate::{Context, Error};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

const SHEETS: &str = "https://sheets.googleapis.com/v4/spreadsheets";
const DOCS: &str = "https://docs.googleapis.com/v1/documents";
/// The rows one read returns.
const ROWS: usize = 1000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    spreadsheet: String,
    range: String,
    values: Option<Vec<Vec<Value>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    document: String,
    text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    title: String,
    text: Option<String>,
}

pub(super) async fn call(cx: &Context, tool: &str, input: Value) -> Result<Value, Error> {
    match tool {
        "sheets_read" => sheets_read(cx, input).await,
        "sheets_update" => write(cx, input, false).await,
        "sheets_append" => write(cx, input, true).await,
        "docs_read" => docs_read(cx, input).await,
        "docs_append" => {
            let document: Document = crate::input(input)?;
            let text = document
                .text
                .ok_or_else(|| Error::Invalid("text is required".into()))?;
            append(cx, id(&document.document), &text).await?;
            Ok(
                json!({"document": id(&document.document), "appended_characters": text.chars().count()}),
            )
        }
        "docs_create" => create(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

async fn sheets_read(cx: &Context, input: Value) -> Result<Value, Error> {
    let range: Range = crate::input(input)?;
    let path = at(SHEETS, &[id(&range.spreadsheet), "values", &range.range]);
    let found = cx.json(cx.get(&path)?).await?;
    let rows = found["values"].as_array().map_or(&[][..], Vec::as_slice);
    Ok(json!({
        "range": found["range"],
        "rows": &rows[..rows.len().min(ROWS)],
        "rows_left_out": rows.len().saturating_sub(ROWS),
    }))
}

/// Values are parsed as if typed into the sheet, so formulas and dates work.
async fn write(cx: &Context, input: Value, append: bool) -> Result<Value, Error> {
    let range: Range = crate::input(input)?;
    let values = range
        .values
        .ok_or_else(|| Error::Invalid("values is required".into()))?;
    let body = json!({"values": values});
    let spreadsheet = id(&range.spreadsheet);
    let request = match append {
        true => {
            let target = format!("{}:append", range.range);
            cx.post(&at(SHEETS, &[spreadsheet, "values", &target]))?
                .query(&[("insertDataOption", "INSERT_ROWS")])
        }
        false => cx.request(
            Method::PUT,
            &at(SHEETS, &[spreadsheet, "values", &range.range]),
        )?,
    };
    let request = request.query(&[("valueInputOption", "USER_ENTERED")]);
    let done = cx.json(request.json(&body)).await?;
    let done = match append {
        true => &done["updates"],
        false => &done,
    };
    Ok(
        json!({"range": done["updatedRange"], "rows": done["updatedRows"], "cells": done["updatedCells"]}),
    )
}

async fn docs_read(cx: &Context, input: Value) -> Result<Value, Error> {
    let document: Document = crate::input(input)?;
    let found = cx
        .json(cx.get(&at(DOCS, &[id(&document.document)]))?)
        .await?;
    let mut text = String::new();
    content(&found["body"]["content"], &mut text);
    Ok(json!({
        "id": found["documentId"],
        "title": found["title"],
        "text": clip(&text, LONGEST_TEXT),
    }))
}

/// The text of structural elements, descending into table cells.
fn content(elements: &Value, text: &mut String) {
    for element in elements.as_array().into_iter().flatten() {
        let runs = element["paragraph"]["elements"].as_array();
        for run in runs.into_iter().flatten() {
            text.push_str(run["textRun"]["content"].as_str().unwrap_or_default());
        }
        let rows = element["table"]["tableRows"].as_array();
        for cell in rows
            .into_iter()
            .flatten()
            .flat_map(|row| row["tableCells"].as_array().into_iter().flatten())
        {
            content(&cell["content"], text);
        }
    }
}

async fn append(cx: &Context, document: &str, text: &str) -> Result<(), Error> {
    let insert = json!({"insertText": {"text": text, "endOfSegmentLocation": {}}});
    let path = format!("{}:batchUpdate", at(DOCS, &[document]));
    let body = json!({"requests": [insert]});
    cx.json(cx.post(&path)?.json(&body)).await?;
    Ok(())
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let made = cx
        .json(cx.post(DOCS)?.json(&json!({"title": create.title})))
        .await?;
    let document = made["documentId"]
        .as_str()
        .ok_or_else(|| Error::Unavailable("Google did not name the new document".into()))?;
    if let Some(text) = create.text.filter(|text| !text.is_empty()) {
        append(cx, document, &text).await?;
    }
    Ok(json!({
        "id": document,
        "title": made["title"],
        "url": format!("https://docs.google.com/document/d/{document}/edit"),
    }))
}
