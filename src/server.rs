use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::sync::RwLock;

use rocket::config::Environment;
use rocket::logger::LoggingLevel;
use rocket::response::content::{Html, JavaScript};
use rocket::{Config, Request, Rocket, State};

use rocket_contrib::json::Json;

use rhai::{Engine, ImmutableString, Module, Scope};

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};

use serde::de::DeserializeOwned;

use rand::*;

use crate::types::{
    APIKeyRequest, EnvInfo, FindHandlerRequest, FindHandlerResponse, GenericOkResponse,
    GithubIssueCreateResponse, Handler, SlackConversationInfoResponse, SlackEvent,
    UpsertHandlerRequest, UserResponse,
};

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
                        println!("\t=> Unexpected error triggered! {}", t.to_string());
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
            "{{ \"channel\": \"{}\", \"text\": \"{}\", \"unfurl_links\": \"true\"}}",
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

fn github_issue_create_internal(
    client: &Client,
    token: &String,
    repo: String,
    title: String,
    body: String,
) -> Option<GithubIssueCreateResponse> {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, format!("token {}", token).parse().unwrap());
    headers.insert(USER_AGENT, HeaderValue::from_static("dti-majordomo"));

    let req: Result<Response, _> = client
        .post(&format!("https://api.github.com/repos/{}/issues", repo))
        .headers(headers)
        .body(format!(
            "{{ \"title\": \"{}\", \"body\": \"{}\"}}",
            title, body
        ))
        .send();

    let resp: Option<GithubIssueCreateResponse> = try_parse_response(req.ok());
    println!("\t=> Github Issue Create: {:?}", resp);
    resp
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
            let addr = handler_addr.clone();
            let slack_post = move |channel: ImmutableString, message: ImmutableString| {
                println!(
                    "\t=> /h/{} made a slack message in channel #{}: {}",
                    addr, channel, message
                );

                Ok(slack_post_internal(
                    &client,
                    &slack_token,
                    channel.into(),
                    message.into(),
                ))
            };

            // Provide a way for Client code to make slack requests
            // Note that the API exposed to clients does not allow them to specify a token
            // That is hidden away, and never exposed to Rhai, so it cannot be leaked
            let client = Client::new();
            let github_token = env.github_token.clone();
            let addr = handler_addr.clone();
            let github_issue_create =
                move |repo: ImmutableString, title: ImmutableString, body: ImmutableString| {
                    println!(
                        "\t=> /h/{} created a new issue in {}, with title: {} and body: {}",
                        addr, repo, title, body
                    );

                    github_issue_create_internal(
                        &client,
                        &github_token,
                        repo.into(),
                        title.into(),
                        body.into(),
                    )
                    .ok_or("Test".into())
                };

            let debug_println = |string: ImmutableString| Ok(println!("{}", string));

            // Register the various functions available to clients
            let mut module = Module::new();
            module.set_fn_2("slack_post", slack_post);
            module.set_fn_3("github_issue_create", github_issue_create);
            module.set_fn_1("debug_println", debug_println);

            let mut engine = Engine::new();
            engine.load_package(module);
            engine.set_max_operations(1000);
            engine
                .register_type::<GithubIssueCreateResponse>()
                .register_get("url", GithubIssueCreateResponse::get_url)
                .register_get("id", GithubIssueCreateResponse::get_id)
                .register_get("title", GithubIssueCreateResponse::get_title);
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

/// Compute if a client is authorized or not
/// TODO documentation
fn check_auth(key: &String, api_keys: Collection<String, ()>) -> bool {
    let guard = api_keys.read().unwrap();
    let map = guard.deref();
    map.contains_key(key)
}

/// Public wrapper around check auth
/// TODO: documentation
/// TODO: Maybe rethink over security policy here
/// TODO: unused, and really should be removed!
/// Is it really a good idea to allow anyone to test if a api key is valid?
/// On the other hand, you can figure this out by calling other methods.
#[post("/verify_key", data = "<post_data>")]
fn verify_key(
    api_keys: Collection<String, ()>,
    post_data: Json<APIKeyRequest>,
) -> Json<UserResponse> {
    match check_auth(&post_data.0.api_key, api_keys) {
        true => Json(UserResponse::success()),
        false => Json(UserResponse::failure("Invalid API Key".into())),
    }
}

/// List handlers
/// TODO: Documentation
/// TODO: rethink security policy here
/// Is it a good idea that anyone with an API Key can see all endpoints?
/// For now, it is...
#[post("/list_handlers", data = "<post_data>")]
fn list_handlers(
    api_keys: Collection<String, ()>,
    handlers: Collection<String, Handler>,
    post_data: Json<APIKeyRequest>
) -> Json<UserResponse> {
    if !check_auth(&post_data.0.api_key, api_keys) {
        return Json(UserResponse::failure("Invalid API Key".into()));
    }

    let guard = handlers.read().unwrap();
    let map = guard.deref();

    let handler_addrs = map.keys().map(String::clone).collect::<Vec<String>>();

    Json(
        UserResponse::success_with_raw(handler_addrs).unwrap_or(UserResponse::failure(
            "Internal Server Error Code 2: Ping Luis Hoderlein about it".into(),
        )),
    )
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

    // fail is user is not auth'd
    if !check_auth(&data.api_key, api_keys) {
        return Json(UserResponse::failure("Invalid API Key".into()));
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

/// Check if a user is

/// Fetch a particular handler
/// TODO documentation
#[post("/find_handler", data = "<post_data>")]
fn find_handler(
    api_keys: Collection<String, ()>,
    handlers: Collection<String, Handler>,
    post_data: Json<FindHandlerRequest>,
) -> Json<UserResponse> {
    let handler = post_data.0.uri;
    let key = post_data.0.api_key;

    // fail is user is not auth'd
    if !check_auth(&key, api_keys) {
        return Json(UserResponse::failure("Invalid API Key".into()));
    }

    let guard = handlers.read().unwrap();
    let map = guard.deref();

    match map.get(&handler) {
        Some(h) => {
            if h.api_key == key {
                Json(
                    UserResponse::success_with_raw(FindHandlerResponse {
                        code: h.code.raw.clone(),
                    })
                    .unwrap_or(UserResponse::failure(
                        "Internal Server Error Code 1: Ping Luis Hoderlein about it".into(),
                    )),
                )
            } else {
                Json(UserResponse::failure("Invalid API Key".into()))
            }
        }
        None => Json(UserResponse::failure("Unknown handler uri".into())),
    }
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

    // TODO Terrible hack to the get the name of the channel that this message was posted in
    // One day, we may get an improved implementation
    // For now, this just works, and that's ok!
    // Alternative 1. Fetch this data once when the app starts
    // Alternative 2. Allow only slack endpoints with the slack id as the uri
    // That would be hard on the user though, and we can't have that!
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
    let first_space = post_data.event.text.find(' ').unwrap_or(0);
    let data = post_data.event.text.clone()[first_space..].to_string();
    let res = call_handler(env, handlers, addr, data);
    if !res.status {
        println!(
            "\t=> Something has errored internally on a slack message: {:?}",
            res.data
        )
    }
}

/// Rocket Endpoint which serves the frontend to any user
#[get("/")]
fn site_root() -> Html<String> {
    if rand::thread_rng().gen_bool(0.5) {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // TODO: maybe handle the result
        let _req: Result<Response, _> = Client::new()
            .post("https://major.ngrok.io/h/awesome-endpoint-2")
            .headers(headers)
            .body("Hey, remember how you have that backend function that might have a critical error condition? Well, it was happened. Now you know!")
            .send();

        Html(include_str!("error_page.html").into())
    } else {
        Html(include_str!("site.html").into())
    }
}

/// Rocket Endpoint which
#[get("/suggestion-box.js")]
fn suggestion_box_js() -> JavaScript<String> {
    JavaScript(include_str!("suggestion-box.js").into())
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
                site_root,
                call_handler,
                upsert_handler,
                slack_redirector,
                list_handlers,
                find_handler,
                verify_key,
                suggestion_box_js
            ],
        )
        .register(catchers![not_found, bad_request, unprocessable_entity])
        .manage(env)
        .manage(RwLock::new(handlers))
        .manage(RwLock::new(api_keys));

    rocket
}
