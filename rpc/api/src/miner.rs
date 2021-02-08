// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2

pub use self::gen_client::Client as MinerClient;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;

#[rpc(client,server,schema)]
pub trait MinerApi {
    /// submit mining seal
    #[rpc(name = "mining.submit")]
    fn submit(&self, minting_blob: String, nonce: u32, extra: String) -> Result<()>;
}
#[test]
fn test() {
    let schema = rpc_impl_MinerApi::gen_client::Client::gen_schema();
    let j = serde_json::to_string_pretty(&schema).unwrap();
    println!("{}", j);
}