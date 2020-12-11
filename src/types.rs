use std::fmt;
use std::fmt::Debug;

use serde::export::Formatter;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use rhai::{Engine, ParseError, AST};

/// A wrapper type which contains immutable state information for the server
pub struct EnvInfo {
    /// The slack token for Majordomo
    pub slack_token: String,
    /// The github token for Majordomo
    pub github_token: String,
    /// The filepath to save the handlers to
    pub handlers_path: String,
}

/// A wrapper type which allows us to serialize and deserialize the AST
pub struct ASTBox {
    pub ast: AST,
    pub raw: String,
}

impl Debug for ASTBox {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "\"{}\"", self.raw)
    }
}

/// Represents a handler, i.e. a Client defined bit of code, which reacts to events
#[derive(Debug, Serialize, Deserialize)]
pub struct Handler {
    /// The URI of the handler, where it is reachable
    pub uri: String,
    /// The API Key associated with the owner of the handler
    pub api_key: String,
    /// A wrapper around the AST and source code for serialization/deserialization purposes
    #[serde(serialize_with = "serialize_astbox")]
    #[serde(deserialize_with = "deserialize_astbox")]
    pub code: ASTBox,
}

impl Handler {
    pub fn new(uri: String, api_key: String, code: String) -> Result<Handler, ParseError> {
        let engine = Engine::new();
        let ast = engine.compile(&code)?;
        Ok(Handler {
            uri,
            api_key,
            code: ASTBox { ast, raw: code },
        })
    }
}

fn serialize_astbox<S: Serializer>(astbox: &ASTBox, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&astbox.raw)
}

fn deserialize_astbox<'de, D: Deserializer<'de>>(d: D) -> Result<ASTBox, D::Error> {
    let code = String::deserialize(d)?;
    let engine = Engine::new();
    let ast = engine
        .compile(&code)
        .map_err(|_| serde::de::Error::custom("Unable to compile!"))?;

    Ok(ASTBox { ast, raw: code })
}

/// Represents a client's request to create/update a handler
#[derive(Debug, Serialize, Deserialize)]
pub struct UpsertHandlerRequest {
    /// The URI of the handler to update
    pub uri: String,
    /// The Client's API Key. Must match the api key specified in the handler
    pub api_key: String,
    /// The new code to push
    pub code: String,
}

/// Represents a client's request to find out more about a handler
#[derive(Debug, Serialize, Deserialize)]
pub struct FindHandlerRequest {
    /// The uri of the handler to find
    pub uri: String,
    /// The API Key associated with the handler. Must match what is present in db!
    pub api_key: String,
}

/// Represents the result of an attempt to find a handler
#[derive(Debug, Serialize, Deserialize)]
pub struct FindHandlerResponse {
    /// The code associated with this handler
    pub code: String,
}

/// Represents a request which takes only an api key
/// E.g. verify_key, list_handlers
#[derive(Debug, Serialize, Deserialize)]
pub struct APIKeyRequest {
    pub api_key: String,
}

/// Represents the response to a User query
#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse {
    /// Represents the status of the operation: True on success, False on failure
    pub status: bool,
    /// Represents an optional bit of additional information present.
    /// On a success, this might be json returned from a handler
    /// On a failure, this is the cause of the failure
    pub data: Option<String>,
}

impl UserResponse {
    pub fn success() -> UserResponse {
        UserResponse {
            status: true,
            data: None,
        }
    }

    pub fn success_with_data(data: String) -> UserResponse {
        UserResponse {
            status: true,
            data: Some(data),
        }
    }

    pub fn success_with_raw<T: Serialize>(value: T) -> Option<UserResponse> {
        let data = serde_json::to_string(&value);
        match data {
            Ok(s) => Some(UserResponse {
                status: true,
                data: Some(s),
            }),
            Err(_) => None,
        }
    }

    pub fn failure(cause: String) -> UserResponse {
        UserResponse {
            status: false,
            data: Some(cause),
        }
    }
}

/// Represents the challenge send by slack
#[derive(Serialize, Deserialize, Debug)]
pub struct SlackVerification {
    pub token: String,
    pub challenge: String,
    #[serde(rename = "type")]
    pub req_type: String,
}

/// Represents the standard
#[derive(Serialize, Deserialize, Debug)]
pub struct SlackEvent {
    pub token: String,
    pub event: SlackEventInner,
    pub event_time: i64,
}

/// Represents the inner event
/// TODO: this only conforms to a text message. Too bad. No emoji reacts yet
#[derive(Serialize, Deserialize, Debug)]
pub struct SlackEventInner {
    #[serde(rename = "type")]
    pub req_type: String,
    pub channel: String,
    pub user: String,
    pub text: String,
    pub ts: String,
}

/// When a response has an Ok, and that ok is all we care about
#[derive(Serialize, Deserialize, Debug)]
pub struct GenericOkResponse {
    pub ok: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SlackConversationInfoResponse {
    pub ok: bool,
    pub channel: SlackConversationInfoResponseInner,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SlackConversationInfoResponseInner {
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GithubIssueCreateResponse {
    pub html_url: String,
    pub title: String,
    pub id: i32,
}

/// Rhai needs this to cooperate
/// Why can't it just read public fields?
/// We may never know
impl GithubIssueCreateResponse {
    pub fn get_url(&mut self) -> String {
        self.html_url.clone()
    }

    pub fn get_title(&mut self) -> String {
        self.title.clone()
    }

    pub fn get_id(&mut self) -> i32 {
        self.id.clone()
    }
}
