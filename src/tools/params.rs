use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileQueryAction {
    Content,
    History,
    Info,
    Metadata,
    Diff,
    Annotations,
    Search,
    Grep,
}

impl FileQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::History => "history",
            Self::Info => "info",
            Self::Metadata => "metadata",
            Self::Diff => "diff",
            Self::Annotations => "annotations",
            Self::Search => "search",
            Self::Grep => "grep",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryFilesParams {
    pub action: FileQueryAction,
    pub file_path: String,
    #[serde(default)]
    pub file2: Option<String>,
    #[serde(default = "default_true")]
    pub diff2: bool,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileModifyAction {
    Add,
    Edit,
    Delete,
    Move,
    Revert,
    Reconcile,
    Resolve,
    Sync,
}

impl FileModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Revert => "revert",
            Self::Reconcile => "reconcile",
            Self::Resolve => "resolve",
            Self::Sync => "sync",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyFilesParams {
    pub action: FileModifyAction,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default = "default_changelist")]
    pub changelist: String,
    #[serde(default)]
    pub source_paths: Option<Vec<String>>,
    #[serde(default)]
    pub target_paths: Option<Vec<String>>,
    #[serde(default = "default_resolve_mode")]
    pub mode: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangelistQueryAction {
    Get,
    List,
}

impl ChangelistQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::List => "list",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryChangelistsParams {
    pub action: ChangelistQueryAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub depot_path: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShelfQueryAction {
    List,
    Diff,
    Files,
}

impl ShelfQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Diff => "diff",
            Self::Files => "files",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryShelvesParams {
    pub action: ShelfQueryAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceQueryAction {
    List,
    Get,
    Type,
    Status,
}

impl WorkspaceQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Get => "get",
            Self::Type => "type",
            Self::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryWorkspacesParams {
    pub action: WorkspaceQueryAction,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobQueryAction {
    ListJobs,
    GetJob,
}

impl JobQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ListJobs => "list_jobs",
            Self::GetJob => "get_job",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryJobsParams {
    pub action: JobQueryAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamQueryAction {
    List,
    Get,
    Children,
    Parent,
    Graph,
    IntegrationStatus,
    GetWorkspace,
    ListWorkspaces,
    ValidateFile,
    ValidateSubmit,
    CheckResolve,
    Interchanges,
}

impl StreamQueryAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Get => "get",
            Self::Children => "children",
            Self::Parent => "parent",
            Self::Graph => "graph",
            Self::IntegrationStatus => "integration_status",
            Self::GetWorkspace => "get_workspace",
            Self::ListWorkspaces => "list_workspaces",
            Self::ValidateFile => "validate_file",
            Self::ValidateSubmit => "validate_submit",
            Self::CheckResolve => "check_resolve",
            Self::Interchanges => "interchanges",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct QueryStreamsParams {
    pub action: StreamQueryAction,
    #[serde(default)]
    pub stream_name: Option<String>,
    #[serde(default)]
    pub stream_path: Option<Vec<String>>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub unloaded: bool,
    #[serde(default)]
    pub all_streams: bool,
    #[serde(default)]
    pub viewmatch: Option<String>,
    #[serde(default)]
    pub view_without_edit: bool,
    #[serde(default)]
    pub at_change: Option<String>,
    #[serde(default)]
    pub both_directions: bool,
    #[serde(default)]
    pub force_refresh: bool,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default)]
    pub changelist: Option<String>,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub long_output: bool,
    #[serde(default)]
    pub limit: Option<u16>,
    #[serde(default = "default_stream_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonQueryParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default = "default_max_results")]
    pub max_results: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct CommonModifyParams {
    pub action: String,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub approval_token: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_max_results() -> u16 {
    100
}

fn default_stream_max_results() -> u16 {
    50
}

fn default_changelist() -> String {
    "default".to_string()
}

fn default_resolve_mode() -> String {
    "auto".to_string()
}
