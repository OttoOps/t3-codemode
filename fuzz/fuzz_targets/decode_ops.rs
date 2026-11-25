#![no_main]
#![allow(dead_code, unused_imports)]

#[path = "../../tests/common/mod.rs"]
mod common;

#[path = "../../tests/all"]
mod all {
    pub mod pr_chaos;
}

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap());
static FILE_STORE: LazyLock<bool> = LazyLock::new(|| std::env::var_os("FUZZ_FILE_STORE").is_some());
static DUMP_OPS: LazyLock<bool> = LazyLock::new(|| std::env::var_os("FUZZ_DUMP_OPS").is_some());

fuzz_target!(|data: &[u8]| {
    RUNTIME.block_on(all::pr_chaos::run_fuzz_bytes(data, !*FILE_STORE, *DUMP_OPS));
});
