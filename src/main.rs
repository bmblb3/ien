use std::env;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use ureq::json;

const ROUTING_SYSTEM_PROMPT: &str = "You are the routing stage of a personal finance Telegram assistant backed by pennywise, a personal finance ledger REST API. Given the user's message and pennywise's OpenAPI schema, pick exactly one GET endpoint that can answer the question, write a jq filter that extracts the relevant data from that endpoint's JSON response, and write a system prompt for a second LLM call that will turn the filtered JSON into a natural-language reply to the user. Only GET endpoints are available — never suggest a write operation.";

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
    api_schema: &Value,
    user_message: &str,
) -> Result<Plan, String> {
    let endpoints = get_endpoints(api_schema);
    if endpoints.is_empty() {
        return Err("pennywise's API schema has no GET endpoints".to_string());
    }

    let body = json!({
        "model": openrouter_model,
        "require_parameters": true,
        "messages": [
            {"role": "system", "content": ROUTING_SYSTEM_PROMPT},
            {"role": "user", "content": format!(
                "User's message: {user_message}\n\nPennywise's OpenAPI schema:\n{api_schema}"
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
    let pennywise_url = env::var("PENNYWISE_URL")
        .expect("set PENNYWISE_URL to the pennywise base URL, e.g. http://pennywise:8080");
    let openrouter_api_key =
        env::var("OPENROUTER_API_KEY").expect("set OPENROUTER_API_KEY to an OpenRouter API key");
    let openrouter_model =
        env::var("OPENROUTER_MODEL").expect("set OPENROUTER_MODEL to an OpenRouter model id");
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

            let Some(chat_id) = update["message"]["chat"]["id"].as_i64() else {
                continue;
            };
            let Some(text) = update["message"]["text"].as_str() else {
                continue;
            };

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

            let reply = match build_plan(&openrouter_api_key, &openrouter_model, &api_schema, text)
            {
                // TODO(#4, #5): execute the plan (GET + jq) and run the Reply Call instead
                // of echoing the plan itself.
                Ok(plan) => format!(
                    "plan: GET {} | jq: {} | reply_system_prompt: {}",
                    plan.endpoint, plan.jq_filter, plan.reply_system_prompt
                ),
                Err(err) => err,
            };

            send_message(&api, chat_id, &reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
