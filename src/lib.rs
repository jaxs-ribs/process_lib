//! kinode process standard library for Rust compiled to WASM
//! Must be used in context of bindings generated by `kinode.wit`.
//!
//! This library provides a set of functions for interacting with the kinode
//! kernel interface, which is a WIT file. The types generated by this file
//! are available in processes via the wit_bindgen macro, if a process needs
//! to use them directly. However, the most convenient way to do most things
//! will be via this library.
//!
//! We define wrappers over the wit bindings to make them easier to use.
//! This library encourages the use of IPC body and metadata types serialized and
//! deserialized to JSON, which is not optimal for performance, but useful
//! for applications that want to maximize composability and introspectability.
//! For blobs, we recommend bincode to serialize and deserialize to bytes.
//!
pub use crate::kinode::process::standard::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

wit_bindgen::generate!({
    path: "kinode-wit",
    world: "lib",
});

/// Interact with the eth provider module.
pub mod eth;
/// Interact with the HTTP server and client modules.
/// Contains types from the `http` crate to use as well.
pub mod http;
/// The types that the kernel itself uses -- warning -- these will
/// be incompatible with WIT types in some cases, leading to annoying errors.
/// Use only to interact with the kernel or runtime in certain ways.
pub mod kernel_types;
/// Interact with the key_value module
pub mod kv;
/// Interact with the sqlite module
pub mod sqlite;
/// Interact with the timer runtime module.
pub mod timer;
/// Interact with the virtual filesystem
pub mod vfs;

// Types

mod package_id;
pub use package_id::PackageId;
mod process_id;
pub use process_id::{ProcessId, ProcessIdParseError};
mod address;
pub use address::{Address, AddressParseError};
mod request;
pub use request::Request;
mod response;
pub use response::Response;
mod message;
use message::wit_message_to_message;
pub use message::{Message, SendError, SendErrorKind};
mod on_exit;
pub use on_exit::OnExit;
mod capability;
pub use capability::Capability;
mod lazy_load_blob;
pub use lazy_load_blob::LazyLoadBlob;

/// Implement the wit-bindgen specific code that the kernel uses to hook into
/// a process. Write an `init(our: Address)` function and call it with this.
#[macro_export]
macro_rules! call_init {
    ($init_func:ident) => {
        struct Component;
        impl Guest for Component {
            fn init(our: String) {
                let our: Address = our.parse().unwrap();
                $init_func(our);
            }
        }
    };
}

/// Override the println! macro to print to the terminal. Uses the
/// `print_to_terminal` function from the WIT interface on maximally-verbose
/// mode, i.e., this print will always show up in the terminal. To control
/// the verbosity, use the `print_to_terminal` function directly.
#[macro_export]
macro_rules! println {
    () => {
        $crate::print_to_terminal(0, "\n");
    };
    ($($arg:tt)*) => {{
        $crate::print_to_terminal(0, &format!($($arg)*));
    }};
}

/// Await the next message sent to this process. The runtime will handle the
/// queueing of incoming messages, and calling this function will provide the next one.
/// Interwoven with incoming messages are errors from the network. If your process
/// attempts to send a message to another node, that message may bounce back with
/// a `SendError`. Those should be handled here.
///
/// TODO: example of usage
pub fn await_message() -> Result<Message, SendError> {
    match crate::receive() {
        Ok((source, message)) => Ok(wit_message_to_message(source, message)),
        Err((send_err, context)) => Err(SendError {
            kind: match send_err.kind {
                crate::kinode::process::standard::SendErrorKind::Offline => SendErrorKind::Offline,
                crate::kinode::process::standard::SendErrorKind::Timeout => SendErrorKind::Timeout,
            },
            message: wit_message_to_message(
                Address::new("our", ProcessId::new(Some("net"), "distro", "sys")),
                send_err.message,
            ),
            lazy_load_blob: send_err.lazy_load_blob,
            context,
        }),
    }
}

/// Simple wrapper over spawn() in WIT to make use of our good types
pub fn spawn(
    name: Option<&str>,
    wasm_path: &str,
    on_exit: OnExit,
    request_capabilities: Vec<Capability>,
    grant_capabilities: Vec<ProcessId>,
    public: bool,
) -> Result<ProcessId, SpawnError> {
    crate::kinode::process::standard::spawn(
        name,
        wasm_path,
        &on_exit._to_standard().map_err(|_e| SpawnError::NameTaken)?,
        &request_capabilities,
        &grant_capabilities,
        public,
    )
}

/// Create a blob with no MIME type and a generic type, plus a serializer
/// function that turns that type into bytes.
///
/// Example: TODO
pub fn make_blob<T, F>(blob: &T, serializer: F) -> anyhow::Result<LazyLoadBlob>
where
    F: Fn(&T) -> anyhow::Result<Vec<u8>>,
{
    Ok(LazyLoadBlob {
        mime: None,
        bytes: serializer(blob)?,
    })
}

/// Fetch the blob of the most recent message we've received. Returns `None`
/// if that message had no blob. If it does have one, attempt to deserialize
/// it from bytes with the provided function.
///
/// Example:
/// ```
/// get_typed_blob(|bytes| Ok(bincode::deserialize(bytes)?)).unwrap_or(MyType {
///     field: HashMap::new(),
///     field_two: HashSet::new(),
/// });
/// ```
pub fn get_typed_blob<T, F>(deserializer: F) -> Option<T>
where
    F: Fn(&[u8]) -> anyhow::Result<T>,
{
    match crate::get_blob() {
        Some(blob) => match deserializer(&blob.bytes) {
            Ok(thing) => Some(thing),
            Err(_) => None,
        },
        None => None,
    }
}

/// Fetch the persisted state blob associated with this process. This blob is saved
/// using the [`set_state`] function. Returns `None` if this process has no saved state.
/// If it does, attempt to deserialize it from bytes with the provided function.
///
/// Example:
/// ```
/// get_typed_state(|bytes| Ok(bincode::deserialize(bytes)?)).unwrap_or(MyStateType {
///     field: HashMap::new(),
///     field_two: HashSet::new(),
/// });
/// ```
pub fn get_typed_state<T, F>(deserializer: F) -> Option<T>
where
    F: Fn(&[u8]) -> anyhow::Result<T>,
{
    match crate::get_state() {
        Some(bytes) => match deserializer(&bytes) {
            Ok(thing) => Some(thing),
            Err(_) => None,
        },
        None => None,
    }
}

/// See if we have the capability to message a certain process.
/// Note if you have not saved the capability, you will not be able to message the other process.
pub fn can_message(address: &Address) -> bool {
    crate::our_capabilities()
        .iter()
        .any(|cap| cap.params == "\"messaging\"" && cap.issuer == *address)
}

/// Get a capability in our store
pub fn get_capability(our: &Address, params: &str) -> Option<Capability> {
    let params = serde_json::from_str::<Value>(params).unwrap_or_default();
    crate::our_capabilities()
        .iter()
        .find(|cap| {
            let cap_params = serde_json::from_str::<Value>(&cap.params).unwrap_or_default();
            cap.issuer == *our && params == cap_params
        })
        .cloned()
}

/// get the next message body from the message queue, or propagate the error
pub fn await_next_message_body() -> Result<Vec<u8>, SendError> {
    match await_message() {
        Ok(msg) => Ok(msg.body().to_vec()),
        Err(e) => Err(e.into()),
    }
}
