use std::env;
use std::thread;
use std::time::Duration;

use ureq::json;

fn main() {
    let token = env::var("TELEGRAM_BOT_TOKEN")
        .expect("set TELEGRAM_BOT_TOKEN to the token from @BotFather");
    let pennywise_url = env::var("PENNYWISE_URL")
        .expect("set PENNYWISE_URL to the pennywise base URL, e.g. http://pennywise:8080");
    let api = format!("https://api.telegram.org/bot{token}");
    let balances_url = format!("{pennywise_url}/balances");

    let mut offset = 0i64;
    loop {
        let updates = match ureq::get(&format!("{api}/getUpdates"))
            .query("timeout", "30")
            .query("offset", &offset.to_string())
            .call()
            .and_then(|res| res.into_json::<serde_json::Value>().map_err(Into::into))
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

            let text = match ureq::get(&balances_url)
                .call()
                .and_then(|res| res.into_string().map_err(Into::into))
            {
                Ok(body) => body,
                Err(err) => format!("balances request failed: {err}"),
            };

            if let Err(err) = ureq::post(&format!("{api}/sendMessage"))
                .send_json(json!({ "chat_id": chat_id, "text": text }))
            {
                eprintln!("sendMessage failed: {err}");
            }
        }
    }
}
