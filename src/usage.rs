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

/// An observation whose `resets_at` has passed describes a window that is over,
/// so t3cm drops it and treats the instance as reporting no usage.
pub trait UsageSource: Send + Sync {
    fn usage<'a>(&'a self, instances: &'a [String]) -> BoxFut<'a, Result<BTreeMap<String, Usage>>>;
}
