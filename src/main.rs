use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde_json::Value;
use ureq::json;

/// The routing system prompt's fixed core: compiled into the binary so the bot
/// works correctly even if no overlay is mounted, instead of depending on a
/// runtime file that could be missing or misconfigured.
const ROUTING_SYSTEM_PROMPT_BASE: &str = include_str!("../prompts/routing.md");

/// Reads a prompt file fresh on every call, so editing prompt wording takes effect
/// on the next message without restarting or recompiling the bot. A missing file
/// means "nothing to add here" and returns an empty string; any other read error
/// (bad permissions, a broken mount, ...) still panics, since that's an actual
/// misconfiguration rather than an absent, optional overlay.
fn read_prompt_file(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => panic!("failed to read prompt file '{path}': {err}"),
    }
}

/// Formats "today" in the given timezone as e.g. "2026-09-13 (Sunday), Europe/Riga",
/// so the Routing Call can resolve relative date phrases ("last month", "this week")
/// against a concrete anchor instead of its own (possibly stale) sense of the date.
fn format_today_at(now: DateTime<Tz>, local_timezone: &str) -> String {
    format!(
        "{} ({}), {local_timezone}",
        now.format("%Y-%m-%d"),
        now.format("%A")
    )
}

struct Plan {
    endpoint: String,
    jq_filter: String,
    reply_system_prompt: String,
}

impl Plan {
    fn from_json_str(content: &str) -> Result<Plan, String> {
        let value: Value = serde_json::from_str(content)
            .map_err(|err| format!("routing call returned malformed JSON: {err}"))?;
        let field = |key: &str| -> Result<String, String> {
            value[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("routing call response missing '{key}'"))
        };
        Ok(Plan {
            endpoint: field("endpoint")?,
            jq_filter: field("jq_filter")?,
            reply_system_prompt: field("reply_system_prompt")?,
        })
    }
}

/// True when `update`'s message was sent by the configured owner, identified by Telegram's
/// per-user `from.id` rather than `chat.id`: `chat.id` only coincides with the sender in a
/// private 1:1 chat, and would wrongly treat every member of a group chat as the owner.
fn is_from_owner(update: &Value, owner_id: i64) -> bool {
    update["message"]["from"]["id"].as_i64() == Some(owner_id)
}

/// Paths pennywise's OpenAPI schema exposes a GET method for.
fn get_endpoints(api_schema: &Value) -> Vec<String> {
    api_schema["paths"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, methods)| methods.get("get").is_some())
        .map(|(path, _)| path.clone())
        .collect()
}

/// JSON Schema constraining the Routing Call's structured output. `endpoint` is enumerated
/// from the live schema so the model can't structurally pick a path pennywise doesn't have.
fn plan_json_schema(endpoints: &[String]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["endpoint", "jq_filter", "reply_system_prompt"],
        "properties": {
            "endpoint": {
                "type": "string",
                "enum": endpoints,
                "description": "The pennywise GET endpoint path that answers the user's question."
            },
            "jq_filter": {
                "type": "string",
                "description": "A jq filter applied to the endpoint's JSON response to extract what answers the user's question, e.g. '.' or 'map(select(.account_name? | test(\"swedbank\"; \"i\")))'."
            },
            "reply_system_prompt": {
                "type": "string",
                "description": "A system prompt for a follow-up LLM call that will turn the filtered JSON into a natural-language reply to the user."
            }
        }
    })
}

fn build_plan(
    openrouter_api_key: &str,
    openrouter_model: &str,
    routing_system_prompt: &str,
    api_schema: &Value,
    user_message: &str,
    today: &str,
) -> Result<Plan, String> {
    let endpoints = get_endpoints(api_schema);
    if endpoints.is_empty() {
        return Err("pennywise's API schema has no GET endpoints".to_string());
    }

    let body = json!({
        "model": openrouter_model,
        "require_parameters": true,
        "messages": [
            {"role": "system", "content": routing_system_prompt},
            {"role": "user", "content": format!(
                "User's message: {user_message}\n\nToday's date: {today}\n\nPennywise's OpenAPI schema:\n{api_schema}"
            )}
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "plan",
                "strict": true,
                "schema": plan_json_schema(&endpoints)
            }
        }
    });

    let response: Value = ureq::post("https://openrouter.ai/api/v1/chat/completions")
        .set("Authorization", &format!("Bearer {openrouter_api_key}"))
        .send_json(body)
        .and_then(|res| res.into_json::<Value>().map_err(Into::into))
        .map_err(|err| format!("routing call failed: {err}"))?;

    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("routing call returned no content: {response}"))?;

    Plan::from_json_str(content)
}

/// The Reply Call: turns a Plan's jq'd JSON output into a final plain-text reply, using the
/// system prompt the Routing Call wrote for exactly this purpose.
fn build_reply(
    openrouter_api_key: &str,
    openrouter_model: &str,
    reply_system_prompt: &str,
    jq_output: &str,
) -> Result<String, String> {
    let body = json!({
        "model": openrouter_model,
        "require_parameters": true,
        "messages": [
            {"role": "system", "content": reply_system_prompt},
            {"role": "user", "content": jq_output}
        ]
    });

    let response: Value = ureq::post("https://openrouter.ai/api/v1/chat/completions")
        .set("Authorization", &format!("Bearer {openrouter_api_key}"))
        .send_json(body)
        .and_then(|res| res.into_json::<Value>().map_err(Into::into))
        .map_err(|err| format!("reply call failed: {err}"))?;

    response["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("reply call returned no content: {response}"))
}

/// Pipes `input` through a real `jq` subprocess running `filter` and returns its stdout.
fn run_jq(input: &str, filter: &str) -> Result<String, String> {
    let mut jq = Command::new("jq")
        .arg(filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn jq: {err}"))?;

    jq.stdin
        .take()
        .expect("jq was spawned with a piped stdin")
        .write_all(input.as_bytes())
        .map_err(|err| format!("failed to write to jq's stdin: {err}"))?;

    let output = jq
        .wait_with_output()
        .map_err(|err| format!("failed to wait for jq: {err}"))?;

    if !output.status.success() {
        return Err(format!(
            "jq filter failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    String::from_utf8(output.stdout).map_err(|err| format!("jq produced invalid UTF-8: {err}"))
}

/// Runs a Plan: GET pennywise at the chosen endpoint, then pipes the response body through
/// the Plan's jq filter.
fn execute_plan(pennywise_url: &str, plan: &Plan) -> Result<String, String> {
    let body = ureq::get(&format!("{pennywise_url}{}", plan.endpoint))
        .call()
        .and_then(|res| res.into_string().map_err(Into::into))
        .map_err(|err| format!("endpoint request failed: {err}"))?;

    run_jq(&body, &plan.jq_filter)
}

/// True when `text` invokes the given slash command, e.g. `/balances` or `/balances@ien_bot`
/// (Telegram appends `@<bot username>` to commands in group chats) with optional trailing
/// arguments. Anything else (plain conversation, a different command) goes through the LLM.
fn is_command(text: &str, command: &str) -> bool {
    let first_word = text.split_whitespace().next().unwrap_or("");
    let name = first_word.split('@').next().unwrap_or("");
    name == command
}

/// Formats pennywise's `GET /balances` response (one row per own account: `account_name`,
/// `balance` in major units, `currency`) as plain text, one account per line.
fn format_balances(balances: &Value) -> Result<String, String> {
    let rows = balances
        .as_array()
        .ok_or_else(|| format!("balances response was not a JSON array: {balances}"))?;

    if rows.is_empty() {
        return Ok("No accounts.".to_string());
    }

    Ok(rows
        .iter()
        .map(|row| {
            let name = row["account_name"].as_str().unwrap_or("?");
            let balance = row["balance"].as_f64().unwrap_or(0.0);
            let currency = row["currency"].as_str().unwrap_or("");
            format!("{name}: {balance:.2} {currency}")
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Handles `/balances` directly against pennywise, bypassing the routing and reply LLM calls
/// entirely: the endpoint and the shape of the reply are both fixed, so there's nothing for
/// an LLM to decide.
fn handle_balances_command(pennywise_url: &str) -> Result<String, String> {
    let balances: Value = ureq::get(&format!("{pennywise_url}/balances"))
        .call()
        .and_then(|res| res.into_json::<Value>().map_err(Into::into))
        .map_err(|err| format!("failed to fetch balances: {err}"))?;

    format_balances(&balances)
}

fn send_message(api: &str, chat_id: i64, text: &str) {
    if let Err(err) = ureq::post(&format!("{api}/sendMessage"))
        .send_json(json!({ "chat_id": chat_id, "text": text }))
    {
        eprintln!("sendMessage failed: {err}");
    }
}

fn main() {
    let token = env::var("TELEGRAM_BOT_TOKEN")
        .expect("set TELEGRAM_BOT_TOKEN to the token from @BotFather");
    let owner_id: i64 = env::var("TELEGRAM_OWNER_ID")
        .expect("set TELEGRAM_OWNER_ID to your Telegram user id, e.g. from @userinfobot")
        .parse()
        .unwrap_or_else(|_| panic!("TELEGRAM_OWNER_ID must be an integer Telegram user id"));
    let pennywise_url = env::var("PENNYWISE_URL")
        .expect("set PENNYWISE_URL to the pennywise base URL, e.g. http://pennywise:8080");
    let openrouter_api_key =
        env::var("OPENROUTER_API_KEY").expect("set OPENROUTER_API_KEY to an OpenRouter API key");
    let openrouter_model =
        env::var("OPENROUTER_MODEL").expect("set OPENROUTER_MODEL to an OpenRouter model id");
    let routing_prompt_path =
        env::var("ROUTING_PROMPT_PATH").unwrap_or_else(|_| "prompts/routing.md".to_string());
    let reply_prompt_path =
        env::var("REPLY_PROMPT_PATH").unwrap_or_else(|_| "prompts/reply.md".to_string());
    let local_timezone =
        env::var("TZ").expect("set TZ to an IANA timezone name, e.g. Europe/Riga");
    let tz: Tz = local_timezone
        .parse()
        .unwrap_or_else(|_| panic!("TZ '{local_timezone}' is not a valid IANA timezone name"));
    let api = format!("https://api.telegram.org/bot{token}");

    let mut offset = 0i64;
    loop {
        let updates = match ureq::get(&format!("{api}/getUpdates"))
            .query("timeout", "30")
            .query("offset", &offset.to_string())
            .call()
            .and_then(|res| res.into_json::<Value>().map_err(Into::into))
        {
            Ok(body) => body,
            Err(err) => {
                eprintln!("getUpdates failed: {err}");
                thread::sleep(Duration::from_secs(5));
                continue;
            }
        };

        for update in updates["result"].as_array().into_iter().flatten() {
            offset = update["update_id"].as_i64().unwrap_or(offset) + 1;

            if !is_from_owner(&update, owner_id) {
                eprintln!(
                    "ignored message from sender {:?}: not the configured owner",
                    update["message"]["from"]["id"]
                );
                continue;
            }

            let Some(chat_id) = update["message"]["chat"]["id"].as_i64() else {
                continue;
            };
            let Some(text) = update["message"]["text"].as_str() else {
                continue;
            };

            if is_command(text, "/balances") {
                let reply = handle_balances_command(&pennywise_url).unwrap_or_else(|err| {
                    eprintln!("chat {chat_id}: {err}");
                    err
                });
                send_message(&api, chat_id, &reply);
                continue;
            }

            let api_schema: Value = match ureq::get(&format!("{pennywise_url}/openapi.json"))
                .call()
                .and_then(|res| res.into_json::<Value>().map_err(Into::into))
            {
                Ok(schema) => schema,
                Err(err) => {
                    send_message(
                        &api,
                        chat_id,
                        &format!("failed to fetch pennywise's API schema: {err}"),
                    );
                    continue;
                }
            };

            let routing_overlay = read_prompt_file(&routing_prompt_path);
            let routing_system_prompt = if routing_overlay.is_empty() {
                ROUTING_SYSTEM_PROMPT_BASE.to_string()
            } else {
                format!("{ROUTING_SYSTEM_PROMPT_BASE}\n\n{routing_overlay}")
            };
            let reply_style = read_prompt_file(&reply_prompt_path);
            let today = format_today_at(Utc::now().with_timezone(&tz), &local_timezone);

            let result: Result<String, String> = build_plan(
                &openrouter_api_key,
                &openrouter_model,
                &routing_system_prompt,
                &api_schema,
                text,
                &today,
            )
            .and_then(|plan| {
                eprintln!(
                    "chat {chat_id}: plan endpoint={:?} jq_filter={:?}",
                    plan.endpoint, plan.jq_filter
                );
                let jq_output = execute_plan(&pennywise_url, &plan)?;
                let reply_system_prompt = format!("{reply_style}\n\n{}", plan.reply_system_prompt);
                build_reply(
                    &openrouter_api_key,
                    &openrouter_model,
                    &reply_system_prompt,
                    &jq_output,
                )
            });

            let reply = result.unwrap_or_else(|err| {
                eprintln!("chat {chat_id}: {err}");
                err
            });
            send_message(&api, chat_id, &reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn format_today_at_includes_date_weekday_and_timezone() {
        let tz: Tz = "Europe/Riga".parse().unwrap();
        let now = Utc
            .with_ymd_and_hms(2026, 9, 13, 8, 0, 0)
            .unwrap()
            .with_timezone(&tz);

        assert_eq!(
            format_today_at(now, "Europe/Riga"),
            "2026-09-13 (Sunday), Europe/Riga"
        );
    }

    #[test]
    fn read_prompt_file_treats_a_missing_file_as_empty() {
        assert_eq!(read_prompt_file("does/not/exist.md"), "");
    }

    #[test]
    fn is_from_owner_matches_the_configured_id() {
        let update = json!({"message": {"from": {"id": 42}}});
        assert!(is_from_owner(&update, 42));
    }

    #[test]
    fn is_from_owner_rejects_a_different_id() {
        let update = json!({"message": {"from": {"id": 7}}});
        assert!(!is_from_owner(&update, 42));
    }

    #[test]
    fn is_from_owner_rejects_a_missing_from_field() {
        let update = json!({"message": {"text": "hi"}});
        assert!(!is_from_owner(&update, 42));
    }

    #[test]
    fn get_endpoints_keeps_only_get_methods() {
        let schema = json!({
            "paths": {
                "/balances": {"get": {}},
                "/accounts": {"get": {}, "post": {}},
                "/accounts/{id}": {"delete": {}}
            }
        });

        let mut endpoints = get_endpoints(&schema);
        endpoints.sort();

        assert_eq!(
            endpoints,
            vec!["/accounts".to_string(), "/balances".to_string()]
        );
    }

    #[test]
    fn plan_json_schema_enumerates_given_endpoints() {
        let endpoints = vec!["/balances".to_string(), "/accounts".to_string()];
        let schema = plan_json_schema(&endpoints);

        assert_eq!(schema["properties"]["endpoint"]["enum"], json!(endpoints));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn plan_from_json_str_parses_all_fields() {
        let content = r#"{"endpoint":"/balances","jq_filter":".","reply_system_prompt":"be nice"}"#;
        let plan = Plan::from_json_str(content).unwrap();

        assert_eq!(plan.endpoint, "/balances");
        assert_eq!(plan.jq_filter, ".");
        assert_eq!(plan.reply_system_prompt, "be nice");
    }

    #[test]
    fn plan_from_json_str_rejects_missing_field() {
        let content = r#"{"endpoint":"/balances","jq_filter":"."}"#;
        assert!(Plan::from_json_str(content).is_err());
    }

    #[test]
    fn run_jq_applies_the_filter() {
        let input = r#"[{"account_name":"Swedbank"},{"account_name":"Revolut"}]"#;
        let result = run_jq(input, r#"map(select(.account_name | test("swedbank"; "i")))"#)
            .unwrap();

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, json!([{"account_name": "Swedbank"}]));
    }

    #[test]
    fn run_jq_reports_an_invalid_filter() {
        assert!(run_jq("{}", "not valid jq").is_err());
    }

    #[test]
    fn is_command_matches_the_bare_command() {
        assert!(is_command("/balances", "/balances"));
    }

    #[test]
    fn is_command_matches_with_bot_username_suffix() {
        assert!(is_command("/balances@ien_bot", "/balances"));
    }

    #[test]
    fn is_command_matches_with_trailing_arguments() {
        assert!(is_command("/balances please", "/balances"));
    }

    #[test]
    fn is_command_rejects_plain_conversation() {
        assert!(!is_command("what are my balances?", "/balances"));
    }

    #[test]
    fn is_command_rejects_a_different_command() {
        assert!(!is_command("/accounts", "/balances"));
    }

    #[test]
    fn format_balances_lists_one_line_per_account() {
        let balances = json!([
            {"account_id": 1, "account_name": "Swedbank", "balance": 1234.5, "currency": "EUR"},
            {"account_id": 2, "account_name": "Revolut", "balance": 0, "currency": "USD"}
        ]);

        assert_eq!(
            format_balances(&balances).unwrap(),
            "Swedbank: 1234.50 EUR\nRevolut: 0.00 USD"
        );
    }

    #[test]
    fn format_balances_reports_no_accounts() {
        assert_eq!(format_balances(&json!([])).unwrap(), "No accounts.");
    }

    #[test]
    fn format_balances_rejects_a_non_array_response() {
        assert!(format_balances(&json!({"error": "oops"})).is_err());
    }

    #[test]
    fn run_jq_null_coalescing_survives_a_null_field() {
        let input = r#"[{"account_name":"Swedbank"},{"account_name":null},{"other":"x"}]"#;
        let result = run_jq(
            input,
            r#"map(select((.account_name? // "") | test("swedbank"; "i")))"#,
        )
        .unwrap();

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, json!([{"account_name": "Swedbank"}]));
    }
}
