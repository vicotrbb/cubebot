//! PostgreSQL, MySQL and MariaDB over their own wire protocols with sqlx, one connection per call.
//! Every statement is prepared, so the server parses exactly one; reads run in a read-only
//! transaction under a statement timeout and answer bounded rows decoded by column type.
use crate::{
    Account, Answer, Auth, Category, Choice, Connector, Context, Error, Field, FieldKind, Level,
    Manifest, PRIVATE_ADDRESS, Risk, Step, ToolSpec,
};
use futures_util::{TryStreamExt, stream::BoxStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{
    Column, Connection, Decode, Executor, Row, Statement, Type, TypeInfo, ValueRef,
    mysql::{MySqlConnectOptions, MySqlConnection, MySqlRow, MySqlSslMode},
    postgres::{PgConnectOptions, PgConnection, PgRow, PgSslMode, PgTypeKind},
    types::{
        Uuid,
        chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc},
    },
};
use std::{fmt::Display, time::Duration};

const ROWS: usize = 100;
const MOST_ROWS: usize = 200;
/// The most JSON a query answers with; rows past it are left out.
const LONGEST: usize = 256 * 1024;
const LONGEST_CELL: usize = 10_000;
const CONNECTING: Duration = Duration::from_secs(10);
/// The server cancels a statement after this; the client gives up a little later.
const STATEMENT_SECONDS: u64 = 30;
const WAITING: Duration = Duration::from_secs(STATEMENT_SECONDS + 15);

pub struct Sql;

static MANIFEST: Manifest = Manifest {
    id: "sql",
    name: "SQL database",
    description: "Query and change a PostgreSQL, MySQL or MariaDB database.",
    category: Category::Data,
    auth: &[Auth::Password {
        fields: &[
            Field {
                key: "username",
                label: "User",
                kind: FieldKind::Text,
                required: true,
                placeholder: "cubebot",
                help: "A database user made for agents, with only the grants they need.",
            },
            Field {
                key: "password",
                label: "Password",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "",
                help: "",
            },
        ],
    }],
    config: &[
        Field {
            key: "engine",
            label: "Database",
            kind: FieldKind::Choice(&[
                Choice {
                    value: "postgres",
                    label: "PostgreSQL",
                },
                Choice {
                    value: "mysql",
                    label: "MySQL or MariaDB",
                },
            ]),
            required: true,
            placeholder: "",
            help: "",
        },
        Field {
            key: "host",
            label: "Host",
            kind: FieldKind::Text,
            required: true,
            placeholder: "db.example.com",
            help: "A name or address the agents' sandboxes can reach.",
        },
        Field {
            key: "port",
            label: "Port",
            kind: FieldKind::Number,
            required: false,
            placeholder: "5432",
            help: "5432 for PostgreSQL and 3306 for MySQL when empty.",
        },
        Field {
            key: "database",
            label: "Database name",
            kind: FieldKind::Text,
            required: true,
            placeholder: "app",
            help: "",
        },
        Field {
            key: "tls",
            label: "TLS",
            kind: FieldKind::Choice(&[
                Choice {
                    value: "verify",
                    label: "Required, with a certificate from a public authority",
                },
                Choice {
                    value: "require",
                    label: "Required, any certificate",
                },
                Choice {
                    value: "prefer",
                    label: "When the server offers it",
                },
                Choice {
                    value: "disable",
                    label: "Off, for a server on a trusted network",
                },
            ]),
            required: false,
            placeholder: "",
            help: "Required, any certificate, when not set: traffic is encrypted but the server is not authenticated.",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_tables",
            description: "Tables and views, in one schema or, for PostgreSQL, every schema but the system ones; for MySQL the connection's database by default.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"schema":{"type":"string"}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "describe_table",
            description: "A table's columns with their types, nullability and defaults.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"table":{"type":"string"},"schema":{"type":"string","description":"The current schema or database when left out."}},"required":["table"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "query",
            description: "Run one read-only SQL statement, such as SELECT, in a read-only transaction stopped after 30 seconds. Answers the columns and up to 200 rows; cast unusual types to text to read them.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"sql":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":200}},"required":["sql"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "execute",
            description: "Run one statement that changes data or schema, such as INSERT, UPDATE, DELETE or CREATE TABLE, and commit it. Answers the rows affected.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"sql":{"type":"string"}},"required":["sql"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Read,
    guide: &[
        Step {
            title: "Make a database user for agents",
            body: "Agents act as this user, so give it only what they need: reading alone unless they should change data. In PostgreSQL, as an administrator, run the statements below with your own password and database; add INSERT, UPDATE, DELETE to the grant only if agents should write.",
            link: None,
            copy: Some(
                "CREATE ROLE cubebot LOGIN PASSWORD 'choose-a-password';\nGRANT CONNECT ON DATABASE app TO cubebot;\nGRANT USAGE ON SCHEMA public TO cubebot;\nGRANT SELECT ON ALL TABLES IN SCHEMA public TO cubebot;",
            ),
        },
        Step {
            title: "Or, in MySQL and MariaDB",
            body: "Run these as an administrator; % lets the user connect from any address, so narrow it to your sandboxes' network when you can.",
            link: None,
            copy: Some(
                "CREATE USER 'cubebot'@'%' IDENTIFIED BY 'choose-a-password';\nGRANT SELECT ON app.* TO 'cubebot'@'%';",
            ),
        },
        Step {
            title: "Choose TLS",
            body: "Managed databases with a certificate from a public authority, such as Neon, Supabase or PlanetScale, work with the first choice. A self-hosted server with its own certificate needs Required, any certificate; Off sends the password in the clear, so keep it to a trusted network.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "Agents act as this database user, and only its grants bound what execute can change. Make a user with the least privilege they need, SELECT alone for reading.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tables {
    schema: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Describe {
    table: String,
    schema: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    sql: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Execute {
    sql: String,
}

impl Connector for Sql {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let mut link = Link::open(cx, true).await?;
            let sql = match link {
                Link::Postgres(_) => "SELECT current_user::text",
                Link::Mysql(_) => "SELECT CURRENT_USER()",
            };
            let table = link.table(cx, sql, &[], 1).await?;
            link.close().await;
            let user = table.rows[0][0].as_str().unwrap_or_default().to_owned();
            let database = cx.setting("database").unwrap_or_default();
            Ok(Account {
                name: format!("{user} on {database}"),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_tables" => tables(cx, input).await,
                "describe_table" => describe(cx, input).await,
                "query" => query(cx, input).await,
                "execute" => execute(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

/// A query's answer: its columns, then each row's values in column order.
#[derive(Serialize)]
struct Table {
    columns: Vec<Value>,
    rows: Vec<Value>,
    truncated: bool,
}

enum Link {
    Postgres(PgConnection),
    Mysql(MySqlConnection),
}

impl Link {
    /// A connection inside a transaction: read-only, or committed by [`Link::commit`].
    async fn open(cx: &Context, read_only: bool) -> Result<Self, Error> {
        let connecting = async {
            let mut link = connect(cx).await?;
            link.begin(read_only).await?;
            Ok::<_, sqlx::Error>(link)
        };
        tokio::time::timeout(CONNECTING, connecting)
            .await
            .map_err(|_| Error::Unavailable("timed out connecting to the database".into()))?
            .map_err(|error| failed(cx, error))
    }

    async fn begin(&mut self, read_only: bool) -> Result<(), sqlx::Error> {
        match self {
            Self::Postgres(link) => {
                let setup = match read_only {
                    true => "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY; BEGIN READ ONLY",
                    false => "BEGIN",
                };
                let timeout = format!("SET statement_timeout = {}", STATEMENT_SECONDS * 1000);
                link.execute(sqlx::raw_sql(&timeout)).await?;
                link.execute(sqlx::raw_sql(setup)).await?;
            }
            Self::Mysql(link) => {
                // MySQL limits SELECTs with max_execution_time; MariaDB has max_statement_time.
                let timeout = format!(
                    "SET SESSION max_execution_time = {}",
                    STATEMENT_SECONDS * 1000
                );
                if link.execute(sqlx::raw_sql(&timeout)).await.is_err() {
                    let timeout = format!("SET SESSION max_statement_time = {STATEMENT_SECONDS}");
                    link.execute(sqlx::raw_sql(&timeout)).await?;
                }
                // A session default too, as DDL commits implicitly and ends the transaction.
                if read_only {
                    link.execute(sqlx::raw_sql("SET SESSION TRANSACTION READ ONLY"))
                        .await?;
                }
                let begin = match read_only {
                    true => "START TRANSACTION READ ONLY",
                    false => "START TRANSACTION",
                };
                link.execute(sqlx::raw_sql(begin)).await?;
            }
        }
        Ok(())
    }

    /// One prepared statement's columns and up to `limit` rows.
    async fn table(
        &mut self,
        cx: &Context,
        sql: &str,
        binds: &[Option<&str>],
        limit: usize,
    ) -> Result<Table, Error> {
        let answered = async {
            match self {
                Self::Postgres(link) => {
                    let statement = link.prepare(sql).await?;
                    let columns = header(statement.columns());
                    let query = binds
                        .iter()
                        .fold(statement.query(), |query, bind| query.bind(*bind));
                    let (rows, truncated) = collect(query.fetch(link), limit, postgres).await?;
                    Ok(Table {
                        columns,
                        rows,
                        truncated,
                    })
                }
                Self::Mysql(link) => {
                    let statement = link.prepare(sql).await?;
                    let columns = header(statement.columns());
                    let query = binds
                        .iter()
                        .fold(statement.query(), |query, bind| query.bind(*bind));
                    let (rows, truncated) = collect(query.fetch(link), limit, mysql).await?;
                    Ok(Table {
                        columns,
                        rows,
                        truncated,
                    })
                }
            }
        };
        tokio::time::timeout(WAITING, answered)
            .await
            .map_err(|_| Error::Unavailable("the database did not answer in time".into()))?
            .map_err(|error| failed(cx, error))
    }

    /// Runs one prepared statement and commits it.
    async fn change(self, sql: &str) -> Result<u64, sqlx::Error> {
        match self {
            Self::Postgres(mut link) => {
                let statement = link.prepare(sql).await?;
                let done = statement.query().execute(&mut link).await?;
                link.execute(sqlx::raw_sql("COMMIT")).await?;
                link.close().await?;
                Ok(done.rows_affected())
            }
            Self::Mysql(mut link) => {
                let statement = link.prepare(sql).await?;
                let done = statement.query().execute(&mut link).await?;
                link.execute(sqlx::raw_sql("COMMIT")).await?;
                link.close().await?;
                Ok(done.rows_affected())
            }
        }
    }

    /// Ends the read-only transaction politely; a failure leaves nothing to undo.
    async fn close(self) {
        let _ = match self {
            Self::Postgres(link) => link.close().await,
            Self::Mysql(link) => link.close().await,
        };
    }
}

/// Where and as whom to connect.
struct Server<'a> {
    host: &'a str,
    port: Option<u16>,
    database: &'a str,
    user: &'a str,
    password: &'a str,
    tls: &'a str,
}

impl<'a> Server<'a> {
    fn of(cx: &'a Context) -> Result<Self, sqlx::Error> {
        let setting = |key: &str| {
            cx.setting(key).map(str::trim).ok_or_else(|| {
                sqlx::Error::Configuration(format!("the connection has no {key}").into())
            })
        };
        let port = &cx.config()["port"];
        let text = port.as_str().map(str::trim).filter(|text| !text.is_empty());
        let number = text.map(str::parse).and_then(Result::ok).or(port.as_u64());
        Ok(Self {
            host: setting("host")?,
            port: number.and_then(|number| u16::try_from(number).ok()),
            database: setting("database")?,
            user: setting("username")?,
            password: cx
                .secret("password")
                .map_err(|_| sqlx::Error::Configuration("the connection has no password".into()))?,
            tls: cx.setting("tls").unwrap_or("require"),
        })
    }

    fn postgres(&self) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(self.host)
            .port(self.port.unwrap_or(5432))
            .database(self.database)
            .username(self.user)
            .password(self.password)
            .application_name("cubebot")
            .ssl_mode(match self.tls {
                "verify" => PgSslMode::VerifyFull,
                "prefer" => PgSslMode::Prefer,
                "disable" => PgSslMode::Disable,
                _ => PgSslMode::Require,
            })
    }

    fn mysql(&self) -> MySqlConnectOptions {
        MySqlConnectOptions::new()
            .host(self.host)
            .port(self.port.unwrap_or(3306))
            .database(self.database)
            .username(self.user)
            .password(self.password)
            .ssl_mode(match self.tls {
                "verify" => MySqlSslMode::VerifyIdentity,
                "prefer" => MySqlSslMode::Preferred,
                "disable" => MySqlSslMode::Disabled,
                _ => MySqlSslMode::Required,
            })
    }
}

async fn connect(cx: &Context) -> Result<Link, sqlx::Error> {
    let server = Server::of(cx)?;
    match cx.setting("engine") {
        Some("postgres") => Ok(Link::Postgres(
            PgConnection::connect_with(&server.postgres()).await?,
        )),
        Some("mysql") => Ok(Link::Mysql(
            MySqlConnection::connect_with(&server.mysql()).await?,
        )),
        _ => Err(sqlx::Error::Configuration(
            "the connection's database is neither postgres nor mysql".into(),
        )),
    }
}

/// A failure worded for the model, with any secret struck, as servers can echo what they were sent.
fn failed(cx: &Context, error: sqlx::Error) -> Error {
    let redact = |text: &str| cx.credential().redact(text);
    match error {
        sqlx::Error::Database(database) => {
            let code = database.code().unwrap_or_default();
            // 28P01 and 28000: PostgreSQL's and MySQL's refused logins.
            if matches!(code.as_ref(), "28P01" | "28000") {
                return Error::Unauthorized;
            }
            Error::Rejected(redact(&format!("{} ({code})", database.message())))
        }
        sqlx::Error::Configuration(problem) => Error::Invalid(redact(&problem.to_string())),
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::Protocol(_) => {
            Error::Unavailable(redact(&format!("could not reach the database: {error}")))
        }
        other => Error::Rejected(redact(&other.to_string())),
    }
}

fn header<C: Column>(columns: &[C]) -> Vec<Value> {
    columns
        .iter()
        .map(|column| json!({"name": column.name(), "type": column.type_info().name()}))
        .collect()
}

/// Rows until `limit` or [`LONGEST`]; whether any were left out.
async fn collect<R: Row>(
    mut rows: BoxStream<'_, Result<R, sqlx::Error>>,
    limit: usize,
    cell: fn(&R, usize) -> Value,
) -> Result<(Vec<Value>, bool), sqlx::Error> {
    let (mut shown, mut size) = (Vec::new(), 0);
    while let Some(row) = rows.try_next().await? {
        let values = Value::Array((0..row.len()).map(|index| cell(&row, index)).collect());
        size += values.to_string().len();
        if shown.len() == limit || (size > LONGEST && !shown.is_empty()) {
            return Ok((shown, true));
        }
        shown.push(values);
    }
    Ok((shown, false))
}

type Decoder<R> = fn(&R, usize) -> Option<Value>;

fn get<T, R>(row: &R, index: usize) -> Option<Value>
where
    R: Row,
    usize: sqlx::ColumnIndex<R>,
    T: for<'r> Decode<'r, R::Database> + Type<R::Database> + Serialize,
{
    row.try_get::<T, _>(index).ok().map(|value| json!(value))
}

fn shown<T, R>(row: &R, index: usize) -> Option<Value>
where
    R: Row,
    usize: sqlx::ColumnIndex<R>,
    T: for<'r> Decode<'r, R::Database> + Type<R::Database> + Display,
{
    row.try_get::<T, _>(index)
        .ok()
        .map(|value| json!(value.to_string()))
}

fn instant<R>(row: &R, index: usize) -> Option<Value>
where
    R: Row,
    usize: sqlx::ColumnIndex<R>,
    DateTime<Utc>: for<'r> Decode<'r, R::Database> + Type<R::Database>,
{
    let at = row.try_get::<DateTime<Utc>, _>(index).ok()?;
    Some(json!(at.to_rfc3339()))
}

/// Binary values as text when they are UTF-8, otherwise in PostgreSQL's `\x` hex form.
fn bytes<R>(row: &R, index: usize) -> Option<Value>
where
    R: Row,
    usize: sqlx::ColumnIndex<R>,
    Vec<u8>: for<'r> Decode<'r, R::Database> + Type<R::Database>,
{
    let bytes = row.try_get::<Vec<u8>, _>(index).ok()?;
    Some(match String::from_utf8(bytes) {
        Ok(text) => json!(text),
        Err(binary) => json!(format!("\\x{}", crate::s3::hex(binary.as_bytes()))),
    })
}

static POSTGRES: &[(&[&str], Decoder<PgRow>)] = &[
    (&["BOOL"], get::<bool, PgRow>),
    (&["INT2"], get::<i16, PgRow>),
    (&["INT4"], get::<i32, PgRow>),
    (&["INT8"], get::<i64, PgRow>),
    (&["FLOAT4"], get::<f32, PgRow>),
    (&["FLOAT8"], get::<f64, PgRow>),
    (&["NUMERIC"], numeric_cell),
    (&["JSON", "JSONB"], get::<Value, PgRow>),
    (&["UUID"], shown::<Uuid, PgRow>),
    (&["DATE"], shown::<NaiveDate, PgRow>),
    (&["TIME"], shown::<NaiveTime, PgRow>),
    (&["TIMESTAMP"], shown::<NaiveDateTime, PgRow>),
    (&["TIMESTAMPTZ"], instant::<PgRow>),
    (&["BYTEA"], bytes::<PgRow>),
    (
        &["TEXT[]", "VARCHAR[]", "NAME[]"],
        get::<Vec<String>, PgRow>,
    ),
    (&["INT4[]"], get::<Vec<i32>, PgRow>),
    (&["INT8[]"], get::<Vec<i64>, PgRow>),
];

static MYSQL: &[(&[&str], Decoder<MySqlRow>)] = &[
    (&["BOOLEAN"], get::<bool, MySqlRow>),
    (
        &["TINYINT", "SMALLINT", "MEDIUMINT", "INT", "BIGINT"],
        get::<i64, MySqlRow>,
    ),
    (
        &[
            "TINYINT UNSIGNED",
            "SMALLINT UNSIGNED",
            "MEDIUMINT UNSIGNED",
            "INT UNSIGNED",
            "BIGINT UNSIGNED",
            "BIT",
        ],
        get::<u64, MySqlRow>,
    ),
    (&["YEAR"], get::<u16, MySqlRow>),
    (&["FLOAT"], get::<f32, MySqlRow>),
    (&["DOUBLE"], get::<f64, MySqlRow>),
    (&["DECIMAL"], decimal),
    (&["JSON"], get::<Value, MySqlRow>),
    (&["DATE"], shown::<NaiveDate, MySqlRow>),
    (&["TIME"], shown::<NaiveTime, MySqlRow>),
    (&["DATETIME"], shown::<NaiveDateTime, MySqlRow>),
    (&["TIMESTAMP"], instant::<MySqlRow>),
    (
        &[
            "BINARY",
            "VARBINARY",
            "BLOB",
            "TINYBLOB",
            "MEDIUMBLOB",
            "LONGBLOB",
        ],
        bytes::<MySqlRow>,
    ),
];

/// A value decoded by its column's type, as text when no decoder fits, or a note to cast it.
fn cell<R>(row: &R, index: usize, decoders: &[(&[&str], Decoder<R>)], fallback: Decoder<R>) -> Value
where
    R: Row,
    usize: sqlx::ColumnIndex<R>,
    String: for<'r> Decode<'r, R::Database> + Type<R::Database>,
{
    if row.try_get_raw(index).map_or(true, |raw| raw.is_null()) {
        return Value::Null;
    }
    let column = &row.columns()[index];
    let name = column.type_info().name();
    let typed = decoders
        .iter()
        .find(|(names, _)| names.contains(&name))
        .map_or_else(
            || get::<String, R>(row, index),
            |(_, decode)| decode(row, index),
        );
    match typed.or_else(|| fallback(row, index)) {
        Some(Value::String(text)) => Value::String(clip(text)),
        Some(value) => value,
        None => json!(format!("({name}: cast it to text to read it)")),
    }
}

fn clip(text: String) -> String {
    match text.char_indices().nth(LONGEST_CELL) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

fn postgres(row: &PgRow, index: usize) -> Value {
    cell(row, index, POSTGRES, enumerated)
}

fn mysql(row: &MySqlRow, index: usize) -> Value {
    cell(row, index, MYSQL, |_, _| None)
}

/// A PostgreSQL enum's binary form is its label.
fn enumerated(row: &PgRow, index: usize) -> Option<Value> {
    let kind = row.columns()[index].type_info().kind();
    if !matches!(kind, PgTypeKind::Enum(_)) {
        return None;
    }
    row.try_get_unchecked::<String, _>(index)
        .ok()
        .map(Value::String)
}

/// MySQL sends a DECIMAL as its digits.
fn decimal(row: &MySqlRow, index: usize) -> Option<Value> {
    row.try_get_unchecked::<String, _>(index)
        .ok()
        .map(Value::String)
}

fn numeric_cell(row: &PgRow, index: usize) -> Option<Value> {
    let raw = row.try_get_raw(index).ok()?;
    numeric(raw.as_bytes().ok()?).map(Value::String)
}

/// PostgreSQL's binary NUMERIC: digit count, weight, sign and scale, then base-10000 digits, the
/// first `weight` places left of the point.
fn numeric(bytes: &[u8]) -> Option<String> {
    let word = |at: usize| {
        let pair = bytes.get(at * 2..at * 2 + 2)?;
        Some(u16::from_be_bytes([pair[0], pair[1]]))
    };
    let (count, weight) = (usize::from(word(0)?), word(1)? as i16);
    let (sign, scale) = (word(2)?, usize::from(word(3)?));
    let special = [(0xC000, "NaN"), (0xD000, "Infinity"), (0xF000, "-Infinity")];
    if let Some((_, name)) = special.iter().find(|(code, _)| *code == sign) {
        return Some((*name).into());
    }
    let digit = |place: i32| {
        let at = i32::from(weight) - place;
        let inside = (0..count as i32).contains(&at);
        inside.then(|| word(4 + at as usize)).flatten().unwrap_or(0)
    };
    let mut text = String::from(if sign == 0x4000 { "-" } else { "" });
    let whole: Vec<String> = (0..=i32::from(weight).max(0))
        .rev()
        .map(|place| match place == i32::from(weight).max(0) {
            true => digit(place).to_string(),
            false => format!("{:04}", digit(place)),
        })
        .collect();
    text.push_str(&whole.concat());
    if scale > 0 {
        let places = scale.div_ceil(4) as i32;
        let fraction: String = (1..=places)
            .map(|place| format!("{:04}", digit(-place)))
            .collect();
        text.push('.');
        text.push_str(&fraction[..scale]);
    }
    Some(text)
}

/// The statement without trailing semicolons. A second statement is refused here as well as by
/// the server, which prepares only one.
fn single(sql: &str, mysql: bool) -> Result<&str, Error> {
    let sql = sql
        .trim()
        .trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    if sql.is_empty() {
        return Err(Error::Invalid("the statement is empty".into()));
    }
    let bytes = sql.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        at = match (bytes[at], bytes.get(at + 1)) {
            (b';', _) => {
                return Err(Error::Invalid(
                    "one statement at a time: call again for the next".into(),
                ));
            }
            (quote @ (b'\'' | b'"' | b'`'), _) => quoted(bytes, at, quote, mysql),
            (b'-', Some(b'-')) => ended(bytes, at, b"\n"),
            (b'#', _) if mysql => ended(bytes, at, b"\n"),
            (b'/', Some(b'*')) => ended(bytes, at + 2, b"*/"),
            (b'$', _) if !mysql => dollar(sql, at),
            _ => at + 1,
        };
    }
    Ok(sql)
}

/// Just past the closing quote; MySQL also escapes with a backslash.
fn quoted(bytes: &[u8], start: usize, quote: u8, backslash: bool) -> usize {
    let mut at = start + 1;
    while at < bytes.len() && bytes[at] != quote {
        at += if backslash && bytes[at] == b'\\' {
            2
        } else {
            1
        };
    }
    at + 1
}

/// Just past `end`, or the end of the text.
fn ended(bytes: &[u8], from: usize, end: &[u8]) -> usize {
    bytes[from..]
        .windows(end.len())
        .position(|window| window == end)
        .map_or(bytes.len(), |found| from + found + end.len())
}

/// Just past a PostgreSQL dollar-quoted string such as `$body$ ... $body$`; `$1` is a parameter.
fn dollar(sql: &str, start: usize) -> usize {
    let rest = &sql[start + 1..];
    let tag_length = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let quoting =
        rest[tag_length..].starts_with('$') && !rest.starts_with(|c: char| c.is_ascii_digit());
    if !quoting {
        return start + 1;
    }
    let tag = &sql[start..start + tag_length + 2];
    ended(sql.as_bytes(), start + tag.len(), tag.as_bytes())
}

fn is_mysql(cx: &Context) -> bool {
    cx.setting("engine") == Some("mysql")
}

async fn read(
    cx: &Context,
    sql: &str,
    binds: &[Option<&str>],
    limit: usize,
) -> Result<Table, Error> {
    let mut link = Link::open(cx, true).await?;
    let table = link.table(cx, sql, binds, limit).await?;
    // A cut-off answer may still be streaming: dropping the connection ends it sooner than
    // draining it would, and the server rolls back the read-only transaction either way.
    if !table.truncated {
        link.close().await;
    }
    Ok(table)
}

async fn tables(cx: &Context, input: Value) -> Result<Value, Error> {
    let schema = crate::input::<Tables>(input)?.schema;
    let sql = match is_mysql(cx) {
        true => {
            "SELECT table_schema AS `schema`, table_name AS `name`, table_type AS `type` FROM information_schema.tables WHERE table_schema = COALESCE(?, DATABASE()) ORDER BY 1, 2 LIMIT 1000"
        }
        false => {
            "SELECT table_schema::text AS schema, table_name::text AS name, table_type::text AS type FROM information_schema.tables WHERE ($1::text IS NULL AND table_schema NOT IN ('pg_catalog', 'information_schema')) OR table_schema = $1 ORDER BY 1, 2 LIMIT 1000"
        }
    };
    let table = read(cx, sql, &[schema.as_deref()], 1000).await?;
    let listed: Vec<Value> = table
        .rows
        .iter()
        .map(|row| json!({"schema": row[0], "name": row[1], "type": row[2]}))
        .collect();
    Ok(json!({"tables": listed}))
}

async fn describe(cx: &Context, input: Value) -> Result<Value, Error> {
    let describe: Describe = crate::input(input)?;
    let sql = match is_mysql(cx) {
        true => {
            "SELECT column_name, column_type, is_nullable, column_default, column_key FROM information_schema.columns WHERE table_name = ? AND table_schema = COALESCE(?, DATABASE()) ORDER BY ordinal_position"
        }
        false => {
            "SELECT column_name::text, data_type::text, is_nullable::text, column_default::text, NULL::text FROM information_schema.columns WHERE table_name = $1 AND table_schema = COALESCE($2, current_schema()) ORDER BY ordinal_position"
        }
    };
    let binds = [Some(describe.table.as_str()), describe.schema.as_deref()];
    let table = read(cx, sql, &binds, 1000).await?;
    if table.rows.is_empty() {
        return Err(Error::NotFound(format!(
            "no table {} the user can see",
            describe.table
        )));
    }
    let columns: Vec<Value> = table
        .rows
        .iter()
        .map(|row| {
            json!({"name": row[0], "type": row[1], "nullable": row[2] == "YES", "default": row[3], "key": row[4]})
        })
        .collect();
    Ok(json!({"table": describe.table, "columns": columns}))
}

async fn query(cx: &Context, input: Value) -> Result<Value, Error> {
    let query: Query = crate::input(input)?;
    let sql = single(&query.sql, is_mysql(cx))?;
    let limit = query.limit.unwrap_or(ROWS).clamp(1, MOST_ROWS);
    let table = read(cx, sql, &[], limit).await?;
    Ok(json!(table))
}

async fn execute(cx: &Context, input: Value) -> Result<Value, Error> {
    let sql = crate::input::<Execute>(input)?.sql;
    let sql = single(&sql, is_mysql(cx))?;
    let link = Link::open(cx, false).await?;
    // Past this point the statement may have run, so a lost answer is reported as uncertain.
    match tokio::time::timeout(WAITING, link.change(sql)).await {
        Ok(Ok(affected)) => Ok(json!({"rows_affected": affected})),
        Ok(Err(sqlx::Error::Database(database))) => {
            Err(failed(cx, sqlx::Error::Database(database)))
        }
        Ok(Err(_)) | Err(_) => Err(Error::Uncertain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Credential;

    #[test]
    fn numeric_values_are_written_out_from_postgres_base_10000_digits() {
        let encoded = |count: u16, weight: i16, sign: u16, scale: u16, digits: &[u16]| {
            let mut bytes = Vec::new();
            for word in [count, weight as u16, sign, scale].iter().chain(digits) {
                bytes.extend(word.to_be_bytes());
            }
            numeric(&bytes).unwrap()
        };
        assert_eq!(encoded(2, 0, 0, 2, &[123, 4500]), "123.45");
        assert_eq!(encoded(1, -1, 0, 3, &[10]), "0.001");
        assert_eq!(encoded(1, 1, 0, 0, &[1]), "10000");
        assert_eq!(encoded(3, 1, 0x4000, 4, &[12, 3456, 7890]), "-123456.7890");
        assert_eq!(encoded(0, 0, 0, 0, &[]), "0");
        assert_eq!(encoded(0, 0, 0xC000, 0, &[]), "NaN");
        assert_eq!(numeric(&[0, 1]), None);
    }

    #[test]
    fn only_one_statement_is_sent_and_quotes_and_comments_may_hold_semicolons() {
        assert_eq!(single("  SELECT 1;; \n", false), Ok("SELECT 1"));
        let quoted =
            "SELECT ';', \"a;b\", $$x;y$$, $tag$ ; $tag$ FROM t -- one; two\n WHERE a = $1 /* ; */";
        assert_eq!(single(quoted, false), Ok(quoted));
        assert_eq!(
            single(r"SELECT 'it\'s;' FROM `t;`", true).map(str::len),
            Ok(25)
        );
        let refused = [
            ("SELECT 1; DELETE FROM t", false),
            ("COMMIT; DROP TABLE t", true),
            ("SELECT $1; SELECT 2", false),
            (r"SELECT 'a\'; DROP TABLE t; -- '", false),
            ("SELECT 1 # note\n; SELECT 2", true),
            ("  ;  ", false),
        ];
        for (sql, mysql) in refused {
            assert!(
                matches!(single(sql, mysql), Err(Error::Invalid(_))),
                "{sql}"
            );
        }
    }

    #[tokio::test]
    async fn bad_settings_and_input_are_refused_before_connecting() {
        let credential = Credential::Fields {
            fields: [("password".into(), "s3cret-password".into())].into(),
        };
        let cx = Context::new(
            json!({"engine": "oracle", "host": "db", "database": "app", "username": "u"}),
            credential.clone(),
        );
        let refused = Sql.check(&cx).await;
        assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        let cx = Context::new(
            json!({"engine": "postgres", "host": "127.0.0.1", "port": 9, "database": "app", "username": "u"}),
            credential,
        );
        for (tool, input) in [
            ("query", json!({"sql": "SELECT 1; SELECT 2"})),
            ("execute", json!({"sql": " ; "})),
            ("describe_table", json!({"name": "t"})),
        ] {
            let refused = Sql.call(&cx, tool, input).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{tool}: {refused:?}"
            );
        }
        let unreachable = Sql.call(&cx, "query", json!({"sql": "SELECT 1"})).await;
        assert!(
            matches!(unreachable, Err(Error::Unavailable(_))),
            "{unreachable:?}"
        );
    }

    /// Every tool against a real server: set `CUBEBOT_LIVE_SQL_ENGINE` (postgres or mysql),
    /// `CUBEBOT_LIVE_SQL_HOST`, `CUBEBOT_LIVE_SQL_PORT`, `CUBEBOT_LIVE_SQL_DATABASE`,
    /// `CUBEBOT_LIVE_SQL_USER`, `CUBEBOT_LIVE_SQL_PASSWORD` and `CUBEBOT_LIVE_SQL_TLS`, then run
    /// `cargo test -p cubebot-connectors live_sql -- --ignored`. It makes and drops a table named
    /// cubebot_live, so the user needs CREATE.
    #[tokio::test]
    #[ignore = "needs a PostgreSQL, MySQL or MariaDB server"]
    async fn live_sql() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_SQL_{key}")).unwrap();
        let credential = Credential::Fields {
            fields: [("password".into(), env("PASSWORD"))].into(),
        };
        let config = json!({"engine": env("ENGINE"), "host": env("HOST"), "port": env("PORT").parse::<u16>().unwrap(), "database": env("DATABASE"), "username": env("USER"), "tls": env("TLS")});
        let cx = Context::new(config, credential);
        let mysql = env("ENGINE") == "mysql";
        println!("check: {:?}", Sql.check(&cx).await.unwrap());
        let call = |tool: &'static str, input: Value| {
            let cx = &cx;
            async move {
                let answer = Sql.call(cx, tool, input).await;
                let shown = format!("{answer:?}");
                println!("{tool}: {}", shown.chars().take(900).collect::<String>());
                answer
            }
        };
        let _ = call("execute", json!({"sql": "DROP TABLE cubebot_live"})).await;
        let create = match mysql {
            true => {
                "CREATE TABLE cubebot_live (id BIGINT PRIMARY KEY, name VARCHAR(40) NOT NULL, price DECIMAL(10,2), flag BOOLEAN, seen DATETIME, at TIMESTAMP NULL, data JSON, raw VARBINARY(8), size INT UNSIGNED)"
            }
            false => {
                "CREATE TABLE cubebot_live (id BIGINT PRIMARY KEY, name VARCHAR(40) NOT NULL, price NUMERIC(10,2), flag BOOLEAN, seen TIMESTAMP, at TIMESTAMPTZ, data JSONB, raw BYTEA, tags TEXT[], key UUID)"
            }
        };
        call("execute", json!({"sql": create})).await.unwrap();
        let insert = match mysql {
            true => {
                "INSERT INTO cubebot_live VALUES (1, 'ada', 12.50, TRUE, '2026-11-06 10:00:00', '2026-11-06 10:00:00', '{\"a\": 1}', x'00ff', 7), (2, 'bob; x', -0.05, FALSE, NULL, NULL, NULL, NULL, NULL)"
            }
            false => {
                "INSERT INTO cubebot_live VALUES (1, 'ada', 12.50, TRUE, '2026-11-06 10:00:00', '2026-11-06 10:00:00+00', '{\"a\": 1}', '\\x00ff', '{x,y}', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11'), (2, 'bob; x', -0.05, FALSE, NULL, NULL, NULL, NULL, NULL, NULL)"
            }
        };
        let inserted = call("execute", json!({"sql": insert})).await.unwrap();
        assert_eq!(inserted["rows_affected"], 2);
        let tables = call("list_tables", json!({})).await.unwrap();
        assert!(tables.to_string().contains("cubebot_live"));
        let described = call("describe_table", json!({"table": "cubebot_live"}))
            .await
            .unwrap();
        assert_eq!(described["columns"][1]["nullable"], false);
        let read = call(
            "query",
            json!({"sql": "SELECT * FROM cubebot_live ORDER BY id;"}),
        )
        .await
        .unwrap();
        let first = &read["rows"][0];
        assert_eq!(
            (&first[0], &first[1], &first[2], &first[3]),
            (&json!(1), &json!("ada"), &json!("12.50"), &json!(true))
        );
        assert_eq!(first[5], "2026-11-06T10:00:00+00:00");
        // MariaDB's JSON is text.
        let data = first[6]
            .as_str()
            .map_or(first[6].clone(), |text| serde_json::from_str(text).unwrap());
        assert_eq!(data, json!({"a": 1}));
        assert_eq!(first[7], "\\x00ff");
        assert_eq!(read["rows"][1][2], "-0.05");
        let limited = call(
            "query",
            json!({"sql": "SELECT id FROM cubebot_live ORDER BY id", "limit": 1}),
        )
        .await
        .unwrap();
        assert_eq!(
            (
                limited["rows"].as_array().unwrap().len(),
                &limited["truncated"]
            ),
            (1, &json!(true))
        );
        let refused = [
            "DELETE FROM cubebot_live",
            "SELECT 1; DELETE FROM cubebot_live",
        ];
        for sql in refused {
            let answer = call("query", json!({"sql": sql})).await;
            assert!(
                matches!(answer, Err(Error::Rejected(_) | Error::Invalid(_))),
                "{sql}: {answer:?}"
            );
        }
        let dropped = call("query", json!({"sql": "DROP TABLE cubebot_live"})).await;
        assert!(matches!(dropped, Err(Error::Rejected(_))), "{dropped:?}");
        let bad = call("query", json!({"sql": "SELECT * FROM no_such_table"})).await;
        assert!(matches!(bad, Err(Error::Rejected(_))), "{bad:?}");
        // Past the statement guard, the server itself prepares only one statement.
        let unguarded = super::read(&cx, "SELECT 1; DELETE FROM cubebot_live", &[], 10).await;
        assert!(
            matches!(unguarded, Err(Error::Rejected(_))),
            "{:?}",
            unguarded.err()
        );
        let count = call(
            "query",
            json!({"sql": "SELECT COUNT(*) AS n FROM cubebot_live"}),
        )
        .await
        .unwrap();
        assert_eq!(count["rows"][0][0], 2);
        if env("TLS") != "disable" {
            let sql = match mysql {
                true => "SHOW SESSION STATUS LIKE 'Ssl_cipher'",
                false => "SELECT 'ssl', ssl::text FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            };
            let tls = call("query", json!({"sql": sql})).await.unwrap();
            let shown = tls["rows"][0][1].as_str().unwrap_or_default();
            assert!(!shown.is_empty() && shown != "false", "{tls}");
        }
        call("execute", json!({"sql": "DROP TABLE cubebot_live"}))
            .await
            .unwrap();
        let mut wrong = cx.config().clone();
        wrong["username"] = json!("nobody");
        let wrong = Context::new(
            wrong,
            Credential::Fields {
                fields: [("password".into(), "not-the-password".into())].into(),
            },
        );
        assert_eq!(Sql.check(&wrong).await, Err(Error::Unauthorized));
    }
}
