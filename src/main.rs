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

mod server;
use server::http_server_start;

mod types;
use types::Handler;

/// The main function of the entire program
///
/// Handles
/// * Loading in environment variables, and setting defaults
/// * Reading in any saved handlers
/// * TODO figure out if self is reachable globally
/// * TODO post about status on slack
/// * Any other future initialization work
fn main() {
    // Load environment variables
    let port = env::var("PORT")
        .ok()
        .map(|s| s.parse::<u16>().ok())
        .flatten()
        .unwrap_or(17760);

    let handlers_path = env::var("HANDLER_PATH").unwrap_or("handlers.json".into());

    let api_keys_path = env::var("API_KEYS_PATH").unwrap_or("api_keys.json".into());

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

    println!("Loaded handlers from {}: {:?}", handlers_path, handlers);
    println!("Loaded in api keys from {}: {:?}", api_keys_path, api_keys);

    http_server_start(handlers_path, handlers, api_keys, port);
}
