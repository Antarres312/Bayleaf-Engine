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

/// A number for a host function, from a native script.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn to_val(value: bayleaf_script_sdk::native::Value) -> rt::Val {
    use bayleaf_script_sdk::native::Value::{F32, F64, I32, I64};
    match value {
        I32(v) => rt::Val::I32(v),
        I64(v) => rt::Val::I64(v),
        F32(v) => rt::Val::F32(v.into()),
        F64(v) => rt::Val::F64(v.into()),
    }
}

/// A host function's result, for a native script.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn from_val(value: &rt::Val) -> bayleaf_script_sdk::native::Value {
    use bayleaf_script_sdk::native::Value::{F32, F64, I32, I64};
    match value {
        rt::Val::I32(v) => I32(*v),
        rt::Val::I64(v) => I64(*v),
        rt::Val::F32(v) => F32((*v).into()),
        rt::Val::F64(v) => F64((*v).into()),
        _ => I32(0),
    }
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
