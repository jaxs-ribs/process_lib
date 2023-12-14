//! Uqbar process standard library for Rust compiled to WASM
//! Must be used in context of bindings generated by `uqbar.wit`.
//!
//! This library provides a set of functions for interacting with the Uqbar
//! kernel interface, which is a WIT file. The types generated by this file
//! are available in processes via the wit_bindgen macro, if a process needs
//! to use them directly. However, the most convenient way to do most things
//! will be via this library.
//!
//! We define wrappers over the wit bindings to make them easier to use.
//! This library encourages the use of IPC and metadata types serialized and
//! deserialized to JSON, which is not optimal for performance, but useful
//! for applications that want to maximize composability and introspectability.
//! For payloads, we recommend bincode to serialize and deserialize to bytes.
//!
pub use crate::uqbar::process::standard::*;
use serde::{Deserialize, Serialize};

wit_bindgen::generate!({
    path: "wit",
    world: "lib",
});

/// Interact with the Uqbar Filesystem. Usually you will not have the
/// capability to do so! Use the VFS or a database app instead.
pub mod filesystem;
/// Interact with the HTTP server and client modules.
/// Contains types from the `http` crate to use as well.
pub mod http;
/// The types that the kernel itself uses -- warning -- these will
/// be incompatible with WIT types in some cases, leading to annoying errors.
/// Use only to interact with the kernel or runtime in certain ways.
pub mod kernel_types;
/// Interact with the timer runtime module.
pub mod timer;

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
pub use message::{Message, SendError, SendErrorKind};
use message::wit_message_to_message;

/// Implement the wit-bindgen specific code that the kernel uses to hook into
/// a process. Write an `init(our: Address)` function and call it with this.
#[macro_export]
macro_rules! call_init {
    ($init_func:ident) => {
        struct Component;
        impl Guest for Component {
            fn init(our: String) {
                let our = Address::from_str(&our).unwrap();
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
                crate::uqbar::process::standard::SendErrorKind::Offline => SendErrorKind::Offline,
                crate::uqbar::process::standard::SendErrorKind::Timeout => SendErrorKind::Timeout,
            },
            message: wit_message_to_message(
                Address {
                    node: "our".to_string(),
                    process: ProcessId {
                        process_name: "net".to_string(),
                        package_name: "sys".to_string(),
                        publisher_node: "uqbar".to_string(),
                    },
                },
                send_err.message,
            ),
            payload: send_err.payload,
            context,
        }),
    }
}

/// Create a payload with no MIME type and a generic type, plus a serializer
/// function that turns that type into bytes.
///
/// Example: TODO
pub fn make_payload<T, F>(payload: &T, serializer: F) -> anyhow::Result<Payload>
where
    F: Fn(&T) -> anyhow::Result<Vec<u8>>,
{
    Ok(Payload {
        mime: None,
        bytes: serializer(payload)?,
    })
}

/// Fetch the payload of the most recent message we've received. Returns `None`
/// if that message had no payload. If it does have one, attempt to deserialize
/// it from bytes with the provided function.
///
/// Example:
/// ```
/// get_typed_payload(|bytes| Ok(bincode::deserialize(bytes)?)).unwrap_or(MyType {
///     field: HashMap::new(),
///     field_two: HashSet::new(),
/// });
/// ```
pub fn get_typed_payload<T, F>(deserializer: F) -> Option<T>
where
    F: Fn(&[u8]) -> anyhow::Result<T>,
{
    match crate::get_payload() {
        Some(payload) => match deserializer(&payload.bytes) {
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

/// Send the capability to message this process to other process(es). This takes an iterator
/// of [`ProcessId`] since capabilities shared this way are only shared locally. To share
/// a capability remotely, first acquire its signed form using [`get_capability`] then
/// attach it to a request/response using [`attach_capability`]. (This will be streamlined
/// in the future!)
///
/// If `our` is not the `Address` of this process, this function will panic, unless you also
/// hold the messaging capability for the given `Address`!
pub fn grant_messaging<I, T>(our: &Address, grant_to: I)
where
    I: IntoIterator<Item = T>,
    T: Into<ProcessId>,
{
    // the kernel will always give us this capability, so this should never ever fail
    let our_messaging_cap = crate::get_capability(our, &"\"messaging\"".into()).unwrap();
    grant_to.into_iter().for_each(|process| {
        crate::share_capability(&process.into(), &our_messaging_cap);
    });
}

/// See if we have the capability to message a certain process.
pub fn can_message(address: &Address) -> bool {
    crate::get_capability(address, &"\"messaging\"".into()).is_some()
}
