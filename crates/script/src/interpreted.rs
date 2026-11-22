//! Scripts interpreted by wasmi, a pure-Rust interpreter: they start at once and run everywhere,
//! the web build included. The editor's play mode always uses it.

pub(crate) use wasmi as rt;

/// What host functions and guest calls fail with.
pub(crate) type Error = wasmi::Error;
/// What defining a host function fails with.
pub(crate) type LinkError = wasmi::errors::LinkerError;

// The same host is compiled once for each runtime, on purpose: `duplicate_mod` guards against doing so by
// mistake, and `include!` would keep the file from rustfmt.
#[allow(clippy::duplicate_mod)]
#[path = "host.rs"]
pub(crate) mod host;

/// An interpreter that meters every call with fuel.
pub(crate) fn engine() -> rt::Engine {
    let mut config = rt::Config::default();
    config.consume_fuel(true);
    rt::Engine::new(&config)
}

pub(crate) fn error(message: impl Into<String>) -> Error {
    Error::new(message.into())
}

/// One instance per store, with memory capped at [`crate::MEMORY_LIMIT`].
pub(crate) fn limits() -> rt::StoreLimits {
    rt::StoreLimitsBuilder::new().memory_size(crate::MEMORY_LIMIT).instances(1).build()
}

pub(crate) fn instantiate<T>(linker: &rt::Linker<T>, store: &mut rt::Store<T>, module: &rt::Module) -> Result<rt::Instance, Error> {
    linker.instantiate_and_start(store, module)
}

pub(crate) fn out_of_fuel(error: &Error) -> bool {
    matches!(error.as_trap_code(), Some(rt::TrapCode::OutOfFuel))
}

/// What a failed guest call says: the trap, or the host function's error.
pub(crate) fn trap_message(error: &Error) -> String {
    error.to_string()
}

#[cfg(test)]
fn test_host() -> host::ScriptHost {
    host::ScriptHost::from_engine(engine())
}

// The same tests run against each runtime.
#[cfg(test)]
#[allow(clippy::duplicate_mod)]
#[path = "tests.rs"]
mod tests;
