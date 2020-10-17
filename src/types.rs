use std::fmt;
use std::fmt::Debug;

use super::serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::rhai::{Engine, ParseError, AST};
use serde::export::Formatter;

/// A Wrapper Type which allows us to serialize and deserialize the AST
pub struct ASTBox(pub AST, pub String);

impl Debug for ASTBox {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "\"{}\"", self.1)
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
            code: ASTBox { 0: ast, 1: code },
        })
    }
}

fn serialize_astbox<S: Serializer>(astbox: &ASTBox, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&astbox.1)
}

fn deserialize_astbox<'de, D: Deserializer<'de>>(d: D) -> Result<ASTBox, D::Error> {
    let code = String::deserialize(d)?;
    let engine = Engine::new();
    let ast = engine
        .compile(&code)
        .map_err(|_| serde::de::Error::custom("Unable to compile!"))?;

    Ok(ASTBox { 0: ast, 1: code })
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

    pub fn failure(cause: String) -> UserResponse {
        UserResponse {
            status: false,
            data: Some(cause),
        }
    }
}
