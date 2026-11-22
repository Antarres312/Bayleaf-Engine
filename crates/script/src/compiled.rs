//! Scripts compiled to machine code by wasmtime (Cranelift) when they load: much faster than the
//! interpreter, with the same ABI, fuel and memory limits and faults. Exported desktop games use it;
//! it does not build for the web.

pub(crate) use wasmtime as rt;

/// What host functions and guest calls fail with.
pub(crate) type Error = wasmtime::Error;
/// What defining a host function fails with.
pub(crate) type LinkError = wasmtime::Error;

// The same host is compiled once for each runtime, on purpose: `duplicate_mod` guards against doing so by
// mistake, and `include!` would keep the file from rustfmt.
#[allow(clippy::duplicate_mod)]
#[path = "host.rs"]
pub(crate) mod host;

/// A compiler whose code meters every call with fuel, as the interpreter does.
pub(crate) fn engine() -> Result<rt::Engine, String> {
    let mut config = rt::Config::new();
    config.consume_fuel(true);
    rt::Engine::new(&config).map_err(|e| format!("the script compiler cannot start: {e}"))
}

pub(crate) fn error(message: impl Into<String>) -> Error {
    Error::msg(message.into())
}

/// One instance per store, with memory capped at [`crate::MEMORY_LIMIT`].
pub(crate) fn limits() -> rt::StoreLimits {
    rt::StoreLimitsBuilder::new().memory_size(crate::MEMORY_LIMIT).instances(1).build()
}

pub(crate) fn instantiate<T: 'static>(linker: &rt::Linker<T>, store: &mut rt::Store<T>, module: &rt::Module) -> Result<rt::Instance, Error> {
    linker.instantiate(store, module)
}

pub(crate) fn out_of_fuel(error: &Error) -> bool {
    matches!(error.downcast_ref::<rt::Trap>(), Some(rt::Trap::OutOfFuel))
}

/// What a failed guest call says: the trap, or the host function's error, without the call stack
/// wasmtime adds, so it reads as the interpreter's does.
pub(crate) fn trap_message(error: &Error) -> String {
    match error.downcast_ref::<rt::Trap>() {
        Some(trap) => trap.to_string(),
        None => error.root_cause().to_string(),
    }
}

#[cfg(test)]
fn test_host() -> host::ScriptHost {
    host::ScriptHost::from_engine(engine().unwrap())
}

// The same tests run against each runtime.
#[cfg(test)]
#[allow(clippy::duplicate_mod)]
#[path = "tests.rs"]
mod tests;
