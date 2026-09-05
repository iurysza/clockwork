use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "clockwork",
    version,
    about = "Schedule agent prompts, local commands, and HTTPS webhooks"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Output as JSON where the command supports it
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create, inspect, enable, run, and remove jobs
    Job {
        #[command(subcommand)]
        command: Box<JobCommands>,
    },

    /// Manage agent commands and their fixed arguments
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },

    /// Read or set configuration
    Config {
        /// Config key
        key: Option<String>,

        /// Config value (set mode)
        value: Option<String>,
    },

    /// Detect AI coding agents and install Clockwork skills for them
    Setup {
        /// Install for a specific agent (claude, codex, cursor, gemini, opencode, pi)
        #[arg(long)]
        agent: Option<String>,

        /// Install for all supported agents, not just detected ones
        #[arg(long)]
        all: bool,

        /// Overwrite existing skill files
        #[arg(long)]
        force: bool,

        /// Show what would be installed without writing files
        #[arg(long)]
        dry_run: bool,

        /// List installed skills and their status (no installation)
        #[arg(long)]
        list: bool,
    },

    /// Repair backend and generated state
    Repair {
        /// Suppress informational output (only show errors)
        #[arg(long)]
        quiet: bool,
    },

    /// Run self-diagnostics and report health issues
    Doctor,

    /// Install and enable the system scheduler timer
    SetupBackend {
        /// Backend to configure: "systemd" or "launchd"
        backend: String,
    },

    /// Run the scheduler daemon in the foreground
    Daemon {
        /// Dispatch interval in seconds (default: 10)
        #[arg(long)]
        interval: Option<u64>,
    },

    /// Upgrade Clockwork to the latest version
    Upgrade {
        /// Force upgrade even if already on latest version
        #[arg(long)]
        force: bool,
    },

    /// Private scheduler and executor process entrypoints
    #[command(hide = true, name = "_internal")]
    Internal {
        #[command(subcommand)]
        command: InternalCommands,
    },
}

#[derive(Subcommand)]
pub enum JobCommands {
    /// Create a managed job in the disabled state
    Create {
        /// Managed job name
        name: String,

        #[command(flatten)]
        definition: DefinitionArgs,

        #[command(flatten)]
        mutation: MutationArgs,
    },

    /// Update a managed job definition
    Update {
        /// Managed job name
        name: String,

        #[command(flatten)]
        definition: DefinitionArgs,

        #[command(flatten)]
        mutation: MutationArgs,
    },

    /// Allow future scheduled runs
    Enable {
        /// Managed job name
        name: String,

        #[command(flatten)]
        mutation: MutationArgs,
    },

    /// Prevent future scheduled runs
    Disable {
        /// Managed job name
        name: String,

        #[command(flatten)]
        mutation: MutationArgs,
    },

    /// Delete an idle managed job and its source
    Delete {
        /// Managed job name
        name: String,

        #[command(flatten)]
        mutation: MutationArgs,
    },

    /// Run an enabled, idle job now
    Trigger {
        /// Managed job name
        name: String,

        #[command(flatten)]
        mutation: MutationArgs,
    },

    /// Validate one managed source, or every managed source
    Validate {
        /// Managed job name
        name: Option<String>,
    },

    /// Show the combined source and runtime state
    Status {
        /// Managed job name
        name: Option<String>,
    },

    /// List managed jobs
    List,

    /// Show run history for a managed job
    History {
        /// Managed job name
        name: String,

        /// Maximum number of records
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Print a managed job's latest log, or a selected run log
    Logs {
        /// Managed job name
        name: String,

        /// Specific run ID
        #[arg(long)]
        run: Option<String>,

        /// Number of lines from the end
        #[arg(long)]
        lines: Option<usize>,
    },
}

#[derive(Args, Debug, Clone, Default)]
pub struct DefinitionArgs {
    /// Schedule: cron, "in 4h", "every 10s", or a future RFC-3339 timestamp
    #[arg(long)]
    pub schedule: Option<String>,

    /// Local command action
    #[arg(long, visible_alias = "run")]
    pub command: Option<String>,

    /// Prompt action text
    #[arg(long)]
    pub prompt: Option<String>,

    /// HTTPS webhook action URL
    #[arg(long)]
    pub webhook: Option<String>,

    /// Use shell execution for a command action
    #[arg(long)]
    pub shell: bool,

    /// Working directory for a command action
    #[arg(long)]
    pub workdir: Option<String>,

    /// Registered agent profile for a prompt action
    #[arg(long)]
    pub profile: Option<String>,

    /// Working directory override for a prompt action
    #[arg(long)]
    pub cwd: Option<String>,

    /// HTTP method for a webhook action
    #[arg(long)]
    pub method: Option<String>,

    /// HTTP header for a webhook action (repeatable, "Key: Value")
    #[arg(long, action = clap::ArgAction::Append)]
    pub header: Vec<String>,

    /// Request body for a webhook action
    #[arg(long)]
    pub body: Option<String>,

    /// Maximum action duration in seconds
    #[arg(long)]
    pub timeout: Option<u64>,

    /// Tag (repeatable). When supplied to update, replaces all tags.
    #[arg(long, action = clap::ArgAction::Append)]
    pub tag: Vec<String>,
}

impl DefinitionArgs {
    pub fn has_changes(&self) -> bool {
        self.schedule.is_some()
            || self.command.is_some()
            || self.prompt.is_some()
            || self.webhook.is_some()
            || self.shell
            || self.workdir.is_some()
            || self.profile.is_some()
            || self.cwd.is_some()
            || self.method.is_some()
            || !self.header.is_empty()
            || self.body.is_some()
            || self.timeout.is_some()
            || !self.tag.is_empty()
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct MutationArgs {
    /// Validate and preview without changing state
    #[arg(long)]
    pub dry_run: bool,

    /// Skip interactive confirmation
    #[arg(long)]
    pub yes: bool,

    /// Apply only to this source, runtime, and referenced profile revision
    #[arg(long)]
    pub if_revision: Option<String>,
}

#[derive(Subcommand)]
pub enum InternalCommands {
    /// Run one scheduler dispatch tick
    Dispatch,

    /// Execute one claimed or manual invocation
    Execute {
        /// Runtime job ID
        job_id: String,

        /// Scheduled-for timestamp (RFC-3339)
        #[arg(long)]
        scheduled_for: String,

        /// Trigger type
        #[arg(long, default_value = "scheduled")]
        trigger: String,

        /// Claimed run ID (scheduled runs only)
        #[arg(long)]
        run_id: Option<String>,
    },

    /// Execute a fallback action after a failed invocation
    ExecFallback {
        /// Runtime job ID
        job_id: String,

        /// Failed run ID
        #[arg(long)]
        failed_run_id: String,

        /// Failed status
        #[arg(long)]
        failed_status: String,

        /// Failed exit code
        #[arg(long, default_value = "")]
        failed_exit_code: String,

        /// Absolute failed log path
        #[arg(long)]
        failed_log_path: String,

        /// Failed scheduled-for timestamp
        #[arg(long)]
        failed_scheduled_for: String,
    },
}

#[derive(Subcommand)]
pub enum AgentCommands {
    /// Add a new agent profile
    Add {
        /// Agent name
        name: String,

        /// Path to agent binary
        #[arg(long)]
        bin: String,

        /// Additional arguments (repeatable)
        #[arg(long, action = clap::ArgAction::Append)]
        arg: Vec<String>,

        /// Pass prompt via stdin instead of argument
        #[arg(long)]
        prompt_stdin: bool,

        /// Working directory for this agent profile
        #[arg(long)]
        cwd: Option<String>,
    },

    /// Remove an agent profile
    Rm {
        /// Agent name
        name: String,
    },

    /// List agent profiles
    List,

    /// Set the default agent
    Default {
        /// Agent name
        name: String,
    },

    /// Auto-detect AI coding agents on PATH and register them
    Detect {
        /// Overwrite profiles for known agents already registered
        #[arg(long)]
        force: bool,
    },
}
