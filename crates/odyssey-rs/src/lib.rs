pub mod cli;

pub use odyssey_rs_bundle as bundle;
pub use odyssey_rs_manifest as manifest;
pub use odyssey_rs_protocol as protocol;
pub use odyssey_rs_runtime as runtime;
pub use odyssey_rs_server as server;

pub fn init_logging() {
    let _ = env_logger::try_init();
}

#[cfg(test)]
mod tests {
    use super::init_logging;

    #[test]
    fn init_logging_is_idempotent() {
        init_logging();
        init_logging();
    }
}
