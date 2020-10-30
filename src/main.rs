#![feature(proc_macro_hygiene, decl_macro)]

#[macro_use]
extern crate rocket;
extern crate rand;
extern crate reqwest;
extern crate rhai;
extern crate rocket_contrib;
extern crate serde;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::iter::FromIterator;
use std::path::Path;

use rocket_contrib::json::Json;

mod server;
use server::http_server_start;

mod types;
use types::Handler;
use types::SlackVerification;

#[post("/slack_redirector", data = "<post_data>")]
fn slack_redirector(post_data: Json<SlackVerification>) -> Json<String> {
    Json(post_data.challenge.clone())
}

/// The main function of the entire program
///
/// Handles
/// * Loading in environment variables, and setting defaults
/// * Reading in any saved handlers
/// * TODO figure out if self is reachable globally
/// * TODO post about status on slack
/// * Any other future initialization work
fn main() {
    // Allow us to respond to challenge slack thing
    // Set CH_MODE=1 to respond to slack challenges
    // Does not start any of the other server stuff, so you'll need to restart without CH_MODE=1 to
    // allow us to actually run the server
    if let Ok(_) = env::var("CH_MODE") {
        let rocket = rocket::ignite().mount("/", routes![slack_redirector]);

        rocket.launch();
    }

    // Load environment variables
    let port = env::var("PORT")
        .ok()
        .map(|s| s.parse::<u16>().ok())
        .flatten()
        .unwrap_or(8000);

    let handlers_path = env::var("HANDLER_PATH").unwrap_or("handlers.json".into());

    let api_keys_path = env::var("API_KEYS_PATH").unwrap_or("api_keys.json".into());

    let slack_token = env::var("SLACK_TOKEN").unwrap_or("no-slack".into());

    if slack_token == "no-slack" {
        println!("No slack token specified! This will disable slack functionality.")
    }

    let github_token = env::var("GITHUB_TOKEN").unwrap_or("no-github".into());

    // Load in any saved handlers
    let handlers_raw_data = fs::read_to_string(Path::new(&handlers_path)).ok();

    if handlers_raw_data.is_none() {
        println!("Warning! Unable to load any handlers!")
    }

    let handlers: HashMap<String, Handler> = handlers_raw_data
        .map(|data| serde_json::from_str(&data).ok())
        .flatten()
        .unwrap_or(HashMap::new());

    // Load in any saved api keys
    let api_keys_raw_data = fs::read_to_string(Path::new(&api_keys_path)).ok();

    if api_keys_raw_data.is_none() {
        println!("Warning! Unable to load any api keys!")
    }

    let api_keys_vec: Vec<String> = api_keys_raw_data
        .map(|data| serde_json::from_str(&data).ok())
        .flatten()
        .unwrap_or(Vec::new());

    let api_keys = HashMap::from_iter(api_keys_vec.iter().map(|i| (i.clone(), ())));

    println!("Loaded {} Handlers from {}", handlers.len(), handlers_path);
    println!("Loaded {} API Keys from {}", api_keys.len(), api_keys_path);

    println!("{:?}", handlers);

    let rocket = http_server_start(
        slack_token,
        github_token,
        handlers_path,
        handlers,
        api_keys,
        port,
    );

    rocket.launch();
}
