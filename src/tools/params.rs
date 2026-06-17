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
#[serde(rename_all = "snake_case")]
pub enum ChangelistModifyAction {
    Create,
    Update,
    Submit,
    Delete,
    MoveFiles,
}

impl ChangelistModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Submit => "submit",
            Self::Delete => "delete",
            Self::MoveFiles => "move_files",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyChangelistsParams {
    pub action: ChangelistModifyAction,
    #[serde(default)]
    pub changelist_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShelfModifyAction {
    Shelve,
    Unshelve,
    Update,
    Delete,
    UnshelveToChangelist,
}

impl ShelfModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Shelve => "shelve",
            Self::Unshelve => "unshelve",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::UnshelveToChangelist => "unshelve_to_changelist",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyShelvesParams {
    pub action: ShelfModifyAction,
    pub changelist_id: String,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default = "default_changelist")]
    pub target_changelist: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceModifyAction {
    Create,
    Delete,
    Update,
    Switch,
}

impl WorkspaceModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Update => "update",
            Self::Switch => "switch",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyWorkspacesParams {
    pub action: WorkspaceModifyAction,
    pub workspace_name: String,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub workspace_description: Option<String>,
    #[serde(default)]
    pub workspace_options: Option<String>,
    #[serde(default)]
    pub workspace_line_end: Option<String>,
    #[serde(default)]
    pub workspace_view: Option<Vec<String>>,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobModifyAction {
    LinkJob,
    UnlinkJob,
}

impl JobModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LinkJob => "link_job",
            Self::UnlinkJob => "unlink_job",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyJobsParams {
    pub action: JobModifyAction,
    pub changelist_id: String,
    pub job_id: String,
    #[serde(default)]
    pub approval_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamModifyAction {
    Create,
    Update,
    Delete,
    EditSpec,
    ResolveSpec,
    RevertSpec,
    ShelveSpec,
    UnshelveSpec,
    Copy,
    Merge,
    Integrate,
    Populate,
    Switch,
    CreateWorkspace,
}

impl StreamModifyAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::EditSpec => "edit_spec",
            Self::ResolveSpec => "resolve_spec",
            Self::RevertSpec => "revert_spec",
            Self::ShelveSpec => "shelve_spec",
            Self::UnshelveSpec => "unshelve_spec",
            Self::Copy => "copy",
            Self::Merge => "merge",
            Self::Integrate => "integrate",
            Self::Populate => "populate",
            Self::Switch => "switch",
            Self::CreateWorkspace => "create_workspace",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct ModifyStreamsParams {
    pub action: StreamModifyAction,
    #[serde(default)]
    pub stream_name: Option<String>,
    #[serde(default)]
    pub stream_type: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub options: Option<String>,
    #[serde(default)]
    pub parent_view: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub remapped: Option<Vec<String>>,
    #[serde(default)]
    pub ignored: Option<Vec<String>>,
    #[serde(default)]
    pub changelist: Option<String>,
    #[serde(default)]
    pub resolve_mode: Option<String>,
    #[serde(default)]
    pub target_changelist: Option<String>,
    #[serde(default)]
    pub parent_stream: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    #[serde(default)]
    pub preview: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub reverse: bool,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub max_files: Option<u16>,
    #[serde(default)]
    pub output_base: bool,
    #[serde(default, rename = "virtual")]
    pub virtual_stream: bool,
    #[serde(default)]
    pub schedule_branch_resolve: bool,
    #[serde(default)]
    pub integrate_around_deleted: bool,
    #[serde(default)]
    pub skip_cherry_picked: bool,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub alt_roots: Option<Vec<String>>,
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
