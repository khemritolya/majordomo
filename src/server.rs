use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::sync::RwLock;

use rocket::config::Environment;
use rocket::logger::LoggingLevel;
use rocket::response::content::Html;
use rocket::response::Redirect;
use rocket::{Config, Request, Rocket, State};

use rocket_contrib::json::Json;

use rhai::{Engine, ImmutableString, Module, Scope};

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::types::{
    EnvInfo, GenericOkResponse, Handler, SlackConversationInfoResponse, SlackEvent,
    UpsertHandlerRequest, UserResponse,
};
use serde::de::DeserializeOwned;

/// A Type Alias to Emulate a Database of type V, indexed by a key type K
/// This is:
/// * Faster than a real db in this use-case
/// * Sufficient for our purposes
type Collection<'a, K, V> = State<'a, RwLock<HashMap<K, V>>>;

fn try_parse_response<T: DeserializeOwned>(req: Option<Response>) -> Option<T> {
    match req {
        Some(r) => match r.text() {
            Ok(text) => {
                println!("{}", text);
                match text.parse() {
                    Ok(v) => serde_json::from_value(v).ok(),
                    Err(t) => {
                        println!("Unexpected error triggered! {}", t.to_string());
                        None
                    }
                }
            }
            Err(_) => None,
        },
        None => None,
    }
}

/// Post a message to Slack
///
/// # Arguments
///
/// * `client` - A reqwest HTTP "client" to make the request. Never seen by Clients
/// * `token` - The slack token to authenticate with. Never seen by Clients
/// * `channel` - The channel to post to. Specified by the Clients
/// * `message` - The message to send. Specified by the Clients
fn slack_post_internal(client: &Client, token: &String, channel: String, message: String) -> bool {
    if token == "no-slack" {
        return false;
    }

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let req: Result<Response, _> = client
        .post("https://slack.com/api/chat.postMessage")
        .headers(headers)
        .body(format!(
            "{{ \"channel\": \"{}\", \"text\": \"{}\"}}",
            channel, message
        ))
        .send();

    let msg: Option<GenericOkResponse> = try_parse_response(req.ok());
    println!("\t=> Slack: {:?}", msg);
    match msg {
        Some(i) => i.ok,
        None => false,
    }
}

/// Rocket Endpoint which passes User Requests onto the Client provided handlers
///
/// # Arguments
///
/// * `env` - Environment variables
/// * `handlers` - A reference to the collection of User created handlers, indexed by their uris
/// * `handler_addr` - The address of the handler that the User has invoked
/// * `post_data` - Any post data that the client has passed alone with the request
#[post("/h/<handler_addr>", data = "<post_data>")]
fn call_handler(
    env: State<EnvInfo>,
    handlers: Collection<String, Handler>,
    handler_addr: String,
    post_data: String,
) -> Json<UserResponse> {
    let guard = handlers.read().unwrap();
    let map = guard.deref();

    match map.get(&handler_addr) {
        Some(handler) => {
            // Provide a way for Client code to make slack requests
            // Note that the API exposed to clients does not allow them to specify a token
            // That is hidden away, and never exposed to Rhai, so it cannot be leaked
            let client = Client::new();
            let slack_token = env.slack_token.clone();
            let slack_post = move |channel: ImmutableString, message: ImmutableString| {
                println!(
                    "\t=> /h/{} made a slack message in channel #{}: {}",
                    handler_addr, channel, message
                );

                Ok(slack_post_internal(
                    &client,
                    &slack_token,
                    channel.into(),
                    message.into(),
                ))
            };

            // Register the various functions available to clients
            let mut module = Module::new();
            module.set_fn_2("slack_post", slack_post);

            let mut engine = Engine::new();
            engine.load_package(module);
            engine.set_max_operations(1000);
            let engine = engine;

            // Run the client's code in response to user request
            let mut scope = Scope::new();
            let result = engine.call_fn(&mut scope, &handler.code.ast, "handle", (post_data,));

            match result {
                Ok(res) => Json(UserResponse::success_with_data(res)),
                Err(e) => {
                    println!("\t=> Error running client code: {}", e);
                    Json(UserResponse::failure("Error running client code!".into()))
                }
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
    env: State<EnvInfo>,
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

    match save_map(&map, &env.handlers_path) {
        Ok(_) => Json(UserResponse::success()),
        Err(_) => {
            println!("\t=> Unable to save db to file!");
            Json(UserResponse::failure("Server error while saving db".into()))
        }
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

/// Accept inbound slack connections
/// Also doubles as an automatic Slack challenge guard responder
/// Just passes on the request to the appropriate handler
#[post("/slack_redirector", data = "<post_data>")]
fn slack_redirector(
    env: State<EnvInfo>,
    handlers: Collection<String, Handler>,
    post_data: Json<SlackEvent>,
) {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {}", &env.slack_token).parse().unwrap(),
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );

    let req: Result<Response, _> = Client::new()
        .post(&format!(
            "https://slack.com/api/conversations.info?channel={}",
            &post_data.event.channel
        ))
        .headers(headers)
        .send();

    let resp: Option<SlackConversationInfoResponse> = try_parse_response(req.ok());
    let name = match resp {
        Some(data) => data.channel.name,
        None => {
            println!("\t=> Failure getting channel information!");
            return;
        }
    };

    let addr = format!("slack-{}", name);
    let res = call_handler(env, handlers, addr, post_data.event.text.clone());
    if !res.status {
        println!("\t=> Something has errored internally on a slack message: {:?}", res.data)
    }
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
/// * `slack_token` - A slack token to work with, or "no-slack"
/// * `github_token` - A github token to work with, or "no-github"
/// * `handlers_path` - The file path to save the handlers to
/// * `handlers` - A map of uris to the handlers that have that uri
/// * `api_keys` - A hash set of api keys. HashMap<T, ()> is basically the same as HashSet<T>
/// * `port` - the port to start the server on
pub fn http_server_start(
    slack_token: String,
    github_token: String,
    handlers_path: String,
    handlers: HashMap<String, Handler>,
    api_keys: HashMap<String, ()>,
    port: u16,
) -> Rocket {
    let config = Config::build(Environment::Staging)
        .log_level(LoggingLevel::Normal)
        .port(port)
        .finalize()
        .unwrap();

    let env = EnvInfo {
        slack_token,
        github_token,
        handlers_path,
    };

    let rocket = rocket::custom(config)
        .mount(
            "/",
            routes![
                root_redirect,
                call_handler,
                upsert_handler,
                slack_redirector
            ],
        )
        .register(catchers![not_found, bad_request, unprocessable_entity])
        .manage(env)
        .manage(RwLock::new(handlers))
        .manage(RwLock::new(api_keys));

    rocket
}
