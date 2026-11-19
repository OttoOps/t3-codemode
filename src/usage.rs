use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::t3::BoxFut;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub left: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

pub trait UsageSource: Send + Sync {
    fn usage<'a>(&'a self, instances: &'a [String]) -> BoxFut<'a, Result<BTreeMap<String, Usage>>>;
}
