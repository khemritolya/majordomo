use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::sync::RwLock;
use std::ops::{Deref, DerefMut};

use super::rocket::config::Environment;
use super::rocket::logger::LoggingLevel;
use super::rocket::response::content::Html;
use super::rocket::response::Redirect;
use super::rocket::{Config, Request, State};

use super::rocket_contrib::json::Json;

use super::rhai::{Engine, Scope};

use super::reqwest::StatusCode;
use super::reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use super::reqwest::blocking;
use super::reqwest::blocking::{Client, Response};

use super::types::{Handler, UpsertHandlerRequest, UserResponse};

/// A Type Alias to Emulate a Database of type V, indexed by a key type K
/// This is:
/// * Faster than a real db in this use-case
/// * Sufficient for our purposes
type Collection<'a, K, V> = State<'a, RwLock<HashMap<K, V>>>;

/// Post a message to Slack
///
/// # Arguments
///
///
fn slack_post_interal(client: Client, token: String, channel: String, message: String) -> bool {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let req: Result<Response, _> = client.post("https://slack.com/api/chat.postMessage")
        .headers(headers)
        .body(format!("{{ \"channel\": \"{}\", \"text\": \"{}\"}}", channel, message))
        .send();

    match req {
        Ok(r) => { println!("{}", r.status()); r.status() == StatusCode::OK},
        Err(e) => { println!("{}", e); false }
    }
}

/// Rocket Endpoint which passes User Requests onto the Client provided handlers
///
/// # Arguments
///
/// * `handlers` - A reference to the collection of User created handlers, indexed by their uris
/// * `handler_addr` - The address of the handler that the User has invoked
/// * `post_data` - Any post data that the client has passed alone with the request
#[post("/h/<handler_addr>", data = "<post_data>")]
fn call_handler(
    handlers: Collection<String, Handler>,
    handler_addr: String,
    post_data: String,
) -> Json<UserResponse> {
    let guard = handlers.read().unwrap();
    let map = guard.deref();

    match map.get(&handler_addr) {
        Some(handler) => {
            let engine = Engine::new();
            let mut scope = Scope::new();

            let result = engine.call_fn(&mut scope, &handler.code.0, "handle", (post_data,));
            match result {
                Ok(res) => Json(UserResponse::success_with_data(res)),
                // TODO log the error, and ping client about it
                Err(_) => Json(UserResponse::failure("Error running client code!".into())),
            }
        }
        None => {
            let cause = format!("Unable to find endpoint {}", handler_addr);
            Json(UserResponse::failure(cause))
        }
    }
}

/// Rocket Endpoint which allows Clients to create and update handlers.
///
/// # Arguments
///
/// * `handlers_path` - The file path that the db should be saved to after update
/// * `api_keys` - A reference to the collection of Client API keys, used to check for auth
/// * `handlers` - A reference to the collection of User created handlers, indexed by their uris
/// * `post_data` - Any post data that the client has passed alone with the request
///
/// Note that `handlers_path`, `handlers`, and `api_keys` are state managed by Rocket, and are
/// **NOT** part of the User's post requests in any way
#[post("/upsert_handler", data = "<post_data>")]
fn upsert_handler(
    handlers_path: State<String>,
    api_keys: Collection<String, ()>,
    handlers: Collection<String, Handler>,
    post_data: Json<UpsertHandlerRequest>,
) -> Json<UserResponse> {
    let data = post_data.0;

    // figure out if the user is auth'd
    let auth = {
        let guard = api_keys.read().unwrap();
        let map = guard.deref();
        map.contains_key(&data.api_key)
    };

    if !auth {
        return Json(UserResponse::failure("Invalid auth token".into()));
    }

    let mut guard = handlers.write().unwrap();
    let map = guard.deref_mut();

    let new_handler = match Handler::new(data.uri.clone(), data.api_key.clone(), data.code) {
        Ok(h) => h,
        Err(e) => return Json(UserResponse::failure(format!("Error parsing code: {}", e))),
    };

    match map.get(&data.uri) {
        Some(handler) => {
            // prevent one Client changing another's endpoint
            if handler.api_key == data.api_key {
                map.insert(data.uri, new_handler);
            } else {
                let cause = format!("A handler with uri {} already exists", handler.api_key);
                return Json(UserResponse::failure(cause));
            }
        }
        None => {
            map.insert(data.uri, new_handler);
        }
    }

    match save_map(&map, handlers_path.inner()) {
        Ok(_) => Json(UserResponse::success()),
        // TODO if this ever happens, ping about it on slack
        Err(_) => Json(UserResponse::failure("Server error while saving db".into())),
    }
}

/// Save the new state of the database to the disk
///
/// It is reasonable, if unfortunate, that we have to keep the mutex locked while doing this.
/// This operation should not take too long, and in any case should occur only when a Client
/// is updating code, which is not often compared to User requests. A several msec delay is
/// acceptable occasionally.
///
/// # Arguments
///
/// * `map` - the database of handlers to save
/// * `path` - the file path to save to.
///            For testing purposes, if equal to "do-not-write", no write occurs.
fn save_map(map: &HashMap<String, Handler>, path: &String) -> Result<(), std::io::Error> {
    if path == "do-not-write" {
        return Ok(());
    }
    let mut file = File::create(path)?;
    file.write_all(serde_json::to_string(map)?.as_ref())?;
    Ok(())
}

/// Rocket Endpoint which redirects any User or Client who wants to use the service to the github
/// Any information they need is there, and there is as yet no reason for them to see the homepage
/// TODO: eventually, this will send the website, but not yet.
#[get("/")]
fn root_redirect() -> Redirect {
    Redirect::to("https://github.com/khemritolya/majordomo")
}

/// Rocket Endpoint which catches any 404's due to User or Client requests.
/// The resulting page lets them know that it is not a valid url
/// It then redirects them to the project github at 10s.
/// See `notfound.html`
///
/// # Arguments
///
/// * `req` - The request that led to a 404
#[catch(404)]
fn not_found(req: &Request) -> Html<String> {
    let uri = format!("{}", req.uri());
    Html(include_str!("notfound.html").replace("<!--uri-link-->", &uri))
}

/// Rocket Endpoint which catches any 400's due to User or Client requests.
///
/// # Arguments
///
/// * `req` - The request that led to a 404
#[catch(400)]
fn bad_request(req: &Request) -> Json<UserResponse> {
    let cause = format!("The request to {} contained malformed data", req.uri());
    Json(UserResponse::failure(cause))
}

/// Rocket Endpoint which catches "Unprocessable Entity" errors
/// In my experience these mean malformed data
///
/// # Arguments
///
/// * `req` - The request that led to a 422
#[catch(422)]
fn unprocessable_entity(req: &Request) -> Json<UserResponse> {
    let cause = format!("The request to {} contained malformed data", req.uri());
    Json(UserResponse::failure(cause))
}

/// Start the Rocket HTTP Server with certain configuration values
///
/// # Arguments
///
/// * `port` - the port to start the server on
pub fn http_server_start(
    handlers_path: String,
    handlers: HashMap<String, Handler>,
    api_keys: HashMap<String, ()>,
    port: u16,
) {
    let config = Config::build(Environment::Staging)
        .log_level(LoggingLevel::Normal)
        .port(port)
        .finalize()
        .unwrap();

    let client = blocking::Client::new();
    //slack_post_interal(client, "".into(), "majordomo-testing-channel".into(), "Hello World!".into());

    let rocket = rocket::custom(config)
        .mount("/", routes![root_redirect, call_handler, upsert_handler])
        .register(catchers![not_found, bad_request, unprocessable_entity])
        .manage(handlers_path)
        .manage(RwLock::new(handlers))
        .manage(RwLock::new(api_keys));

    rocket.launch();
}
