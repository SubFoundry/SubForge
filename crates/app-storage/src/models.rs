use app_common::ProxyNode;
#[derive(Debug, Clone, PartialEq)]
pub struct NodeCacheEntry {
    pub id: String,
    pub source_instance_id: String,
    pub nodes: Vec<ProxyNode>,
    pub fetched_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshJob {
    pub id: String,
    pub source_instance_id: String,
    pub trigger_type: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub node_count: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportToken {
    pub id: String,
    pub profile_id: String,
    pub token: String,
    pub token_type: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}
