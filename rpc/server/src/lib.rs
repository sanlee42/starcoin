// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2

#[macro_use]
extern crate trace_time;
mod actor;
pub mod module;
mod service;

pub use actor::RpcActor;
