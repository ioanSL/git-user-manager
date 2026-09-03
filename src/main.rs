//! gum — manage multiple git identities and switch between them.

mod actions;
mod doctor;
mod git;
mod profile;
mod signers;
mod sshconfig;
mod tui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::{self, Write};

use git::Scope;
use profile::{Profile, Registry, Signing, Ssh};

#[derive(Parser)]
#[command(
    name = "gum",
    about = "Manage multiple git identities (GitHub/GitLab users)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Add a new profile to the registry.
    Add(AddArgs),
    /// List all profiles.
    List,
    /// Show one profile's full settings.
    Show { name: String },
    /// Show the git identity currently in effect (here, or in a scope).
    Current {
        /// Inspect a specific scope instead of the effective value.
        #[arg(long, value_enum)]
        scope: Option<ScopeArg>,
    },
    /// Apply a profile's identity to a config scope (default: global).
    Use {
        name: String,
        /// Write to the current repo's .git/config instead of ~/.gitconfig.
        #[arg(long)]
        local: bool,
        /// Apply without the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Set a profile as the global default identity (identity + signing only;
    /// no transport — that stays per-repo via auto-switch).
    Default { name: String },
    /// Remove a profile (and disable its auto-switch wiring).
    Remove { name: String },
    /// Manage `hasconfig` auto-switching via includeIf.
    Auto {
        #[command(subcommand)]
        action: AutoAction,
    },
    /// Audit global git config for security/consistency issues.
    Doctor {
        /// Apply available fixes (prompts for each).
        #[arg(long)]
        fix: bool,
        /// Apply fixes without prompting (implies --fix).
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Manage SSH host aliases in ~/.ssh/config.
    Ssh {
        #[command(subcommand)]
        action: SshAction,
    },
    /// Manage the SSH allowed-signers file (verifies SSH-signed commits).
    Signers {
        #[command(subcommand)]
        action: SignersAction,
    },
    /// Launch the interactive terminal UI (also the default with no command).
    Tui,
}

#[derive(Subcommand)]
enum SshAction {
    /// Rewrite all gum-managed ~/.ssh/config blocks from the registry.
    Sync,
    /// Show which profiles have a managed SSH alias.
    Status,
}

#[derive(Subcommand)]
enum SignersAction {
    /// Rebuild the allowed-signers file and wire gpg.ssh.allowedSignersFile.
    Sync,
    /// Show the resolved signer entries and wiring status.
    Status,
}

#[derive(clap::Args)]
struct AddArgs {
    /// Short slug for the profile, e.g. "work".
    name: String,
    #[arg(long = "user-name")]
    user_name: String,
    #[arg(long)]
    email: String,
    /// Credential host base, e.g. "https://github.com".
    #[arg(long)]
    host: Option<String>,
    /// Account username for HTTPS credential disambiguation.
    #[arg(long)]
    username: Option<String>,
    /// Remote glob for auto-switch, e.g. "https://github.com/work-org/**".
    #[arg(long = "remote")]
    remote_match: Option<String>,
    /// Signing format: "ssh" or "openpgp".
    #[arg(long = "sign-format")]
    sign_format: Option<String>,
    /// Signing key (GPG id, or path to an SSH public key).
    #[arg(long = "sign-key")]
    sign_key: Option<String>,
    /// Auto-sign commits and tags for this profile.
    #[arg(long = "auto-sign")]
    auto_sign: bool,
    /// SSH private key path, e.g. ~/.ssh/id_work (enables the SSH layer).
    #[arg(long = "ssh-key")]
    ssh_key: Option<String>,
    /// SSH host alias to manage in ~/.ssh/config, e.g. github.com-work.
    /// With it, SSH remotes route through the alias; without it, core.sshCommand is used.
    #[arg(long = "ssh-host-alias")]
    ssh_host_alias: Option<String>,
    /// Real host the alias targets (default github.com).
    #[arg(long = "ssh-hostname")]
    ssh_hostname: Option<String>,
}

#[derive(Subcommand)]
enum AutoAction {
    /// Wire up includeIf so this profile activates by remote URL.
    Enable { name: String },
    /// Remove this profile's includeIf wiring.
    Disable { name: String },
    /// Show which profiles have auto-switch enabled.
    Status,
}

#[derive(Copy, Clone, clap::ValueEnum)]
enum ScopeArg {
    Global,
    Local,
}

impl From<ScopeArg> for Scope {
    fn from(s: ScopeArg) -> Self {
        match s {
            ScopeArg::Global => Scope::Global,
            ScopeArg::Local => Scope::Local,
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None | Some(Command::Tui) => tui::launch(),
        Some(Command::Add(args)) => cmd_add(args),
        Some(Command::List) => cmd_list(),
        Some(Command::Show { name }) => cmd_show(&name),
        Some(Command::Current { scope }) => cmd_current(scope.map(Into::into)),
        Some(Command::Use { name, local, yes }) => {
            let scope = if local { Scope::Local } else { Scope::Global };
            cmd_use(&name, scope, yes)
        }
        Some(Command::Default { name }) => cmd_default(&name),
        Some(Command::Remove { name }) => cmd_remove(&name),
        Some(Command::Auto { action }) => match action {
            AutoAction::Enable { name } => cmd_auto_enable(&name),
            AutoAction::Disable { name } => cmd_auto_disable(&name),
            AutoAction::Status => cmd_auto_status(),
        },
        Some(Command::Doctor { fix, yes }) => doctor::run(fix || yes, yes, confirm),
        Some(Command::Ssh { action }) => match action {
            SshAction::Sync => cmd_ssh_sync(),
            SshAction::Status => cmd_ssh_status(),
        },
        Some(Command::Signers { action }) => match action {
            SignersAction::Sync => cmd_signers_sync(),
            SignersAction::Status => cmd_signers_status(),
        },
    }
}

fn cmd_add(args: AddArgs) -> Result<()> {
    let signing = match (args.sign_format, args.sign_key) {
        (Some(format), Some(key)) => Some(Signing {
            format,
            key,
            auto_sign: args.auto_sign,
        }),
        (None, None) => None,
        _ => bail!("--sign-format and --sign-key must be provided together"),
    };
    let ssh = args.ssh_key.map(|key| Ssh {
        key,
        hostname: args.ssh_hostname,
        host_alias: args.ssh_host_alias,
    });
    let profile = Profile {
        name: args.name.clone(),
        user_name: args.user_name,
        user_email: args.email,
        host: args.host,
        username: args.username,
        remote_match: args.remote_match,
        signing,
        ssh,
    };
    let mut reg = Registry::load()?;
    reg.add(profile)?;
    reg.save()?;
    // Keep ~/.ssh/config and the allowed-signers file in sync.
    let added = reg.get(&args.name).expect("just added");
    sshconfig::apply_profile(added)?;
    signers::sync(&reg.profiles)?;
    println!("Added profile '{}'.", args.name);
    Ok(())
}

fn cmd_list() -> Result<()> {
    let reg = Registry::load()?;
    if reg.profiles.is_empty() {
        println!("No profiles yet. Add one with: gum add <name> --user-name <n> --email <e>");
        return Ok(());
    }
    for p in &reg.profiles {
        let auto = match &p.remote_match {
            Some(m) => format!("  ⇄ {m}"),
            None => String::new(),
        };
        println!("{:<12} {} <{}>{}", p.name, p.user_name, p.user_email, auto);
    }
    Ok(())
}

fn cmd_show(name: &str) -> Result<()> {
    let reg = Registry::load()?;
    let p = reg
        .get(name)
        .with_context(|| format!("no profile named '{name}'"))?;
    println!("name:         {}", p.name);
    println!("user.name:    {}", p.user_name);
    println!("user.email:   {}", p.user_email);
    if let Some(h) = &p.host {
        println!("host:         {h}");
    }
    if let Some(u) = &p.username {
        println!("username:     {u}");
    }
    if let Some(m) = &p.remote_match {
        println!("remote-match: {m}");
    }
    if let Some(s) = &p.signing {
        println!(
            "signing:      {} key={} auto-sign={}",
            s.format, s.key, s.auto_sign
        );
    }
    if let Some(ssh) = &p.ssh {
        match &ssh.host_alias {
            Some(alias) => println!(
                "ssh:          alias {alias} -> {} key={}",
                actions::ssh_hostname(ssh),
                ssh.key
            ),
            None => println!("ssh:          core.sshCommand key={}", ssh.key),
        }
    }
    Ok(())
}

fn cmd_current(scope: Option<Scope>) -> Result<()> {
    let (name, email) = match scope {
        Some(s) => {
            if s == Scope::Local && !git::in_repo() {
                bail!("not inside a git repository; cannot read local scope");
            }
            (git::get(s, "user.name")?, git::get(s, "user.email")?)
        }
        None => (
            git::get_effective("user.name")?,
            git::get_effective("user.email")?,
        ),
    };
    let where_ = scope.map(|s| s.label()).unwrap_or("effective");
    println!(
        "[{}] {} <{}>",
        where_,
        name.as_deref().unwrap_or("(unset)"),
        email.as_deref().unwrap_or("(unset)")
    );

    // If the effective identity matches a known profile, name it.
    if scope.is_none() {
        if let Some(email) = email {
            let reg = Registry::load()?;
            if let Some(p) = reg.profiles.iter().find(|p| p.user_email == email) {
                println!("matches profile: {}", p.name);
            }
        }
    }
    Ok(())
}

fn cmd_use(name: &str, scope: Scope, yes: bool) -> Result<()> {
    if scope == Scope::Local && !git::in_repo() {
        bail!("not inside a git repository; run from a repo or use the global scope");
    }
    let reg = Registry::load()?;
    let p = reg
        .get(name)
        .with_context(|| format!("no profile named '{name}'"))?;
    let settings = actions::profile_settings(p);

    // Preview: show old -> new for every key we will write.
    println!("Applying '{}' to {}:", name, scope.label());
    for (key, value) in &settings {
        let current = git::get(scope, key)?.unwrap_or_else(|| "(unset)".to_string());
        if &current == value {
            println!("  {key} = {value}  (unchanged)");
        } else {
            println!("  {key}: {current} -> {value}");
        }
    }

    if !yes && !confirm("Apply these changes?")? {
        println!("Aborted.");
        return Ok(());
    }
    for (key, value) in &settings {
        git::set(scope, key, value)?;
    }
    println!("Done.");
    Ok(())
}

fn cmd_default(name: &str) -> Result<()> {
    let reg = Registry::load()?;
    let p = reg
        .get(name)
        .with_context(|| format!("no profile named '{name}'"))?;
    actions::set_default(p, &reg.profiles)?;
    // Keep the verification file current (identity may now sign by default).
    signers::sync(&reg.profiles)?;
    println!("Set '{name}' as the global default identity.");
    println!(
        "  identity + signing applied to ~/.gitconfig; transport stays per-repo via auto-switch."
    );
    Ok(())
}

fn cmd_remove(name: &str) -> Result<()> {
    let mut reg = Registry::load()?;
    let removed = reg.remove(name)?;
    // Best-effort: tear down any auto-switch wiring and include file.
    if let Some(glob) = &removed.remote_match {
        let key = git::auto_key(glob);
        git::unset(Scope::Global, &key)?;
    }
    let include = Registry::include_path(name)?;
    if include.exists() {
        std::fs::remove_file(&include).ok();
    }
    // Drop any managed ~/.ssh/config alias block.
    sshconfig::remove(name)?;
    reg.save()?;
    // Refresh allowed-signers now that the registry changed.
    signers::sync(&reg.profiles)?;
    println!("Removed profile '{name}'.");
    Ok(())
}

fn cmd_ssh_sync() -> Result<()> {
    let reg = Registry::load()?;
    let n = sshconfig::sync(&reg.profiles)?;
    let path = sshconfig::config_path()?;
    println!("Synced {n} alias block(s) into {}.", path.display());
    Ok(())
}

fn cmd_ssh_status() -> Result<()> {
    let reg = Registry::load()?;
    let managed = sshconfig::managed(&reg.profiles)?;
    if managed.is_empty() {
        println!("No managed SSH aliases.");
        return Ok(());
    }
    for (name, alias) in managed {
        println!("{name:<12} {alias}");
    }
    Ok(())
}

fn cmd_signers_sync() -> Result<()> {
    let reg = Registry::load()?;
    let r = signers::sync(&reg.profiles)?;
    println!("Wrote {} signer(s) to {}.", r.written, r.path.display());
    if r.skipped > 0 {
        println!(
            "  {} profile(s) skipped — SSH public key not found.",
            r.skipped
        );
    }
    if let Some(foreign) = r.foreign {
        println!("  note: gpg.ssh.allowedSignersFile already points at {foreign};");
        println!("        left it as-is. Point it at the file above to use these entries.");
    } else if r.wired {
        println!("  gpg.ssh.allowedSignersFile is wired to this file.");
    }
    Ok(())
}

fn cmd_signers_status() -> Result<()> {
    let reg = Registry::load()?;
    let entries = signers::entries(&reg.profiles);
    if entries.is_empty() {
        println!("No SSH signer entries.");
        return Ok(());
    }
    for e in &entries {
        println!(
            "{:<28} {} {}…",
            e.email,
            e.keytype,
            &e.b64[..e.b64.len().min(16)]
        );
    }
    Ok(())
}

fn cmd_auto_enable(name: &str) -> Result<()> {
    let reg = Registry::load()?;
    let p = reg
        .get(name)
        .with_context(|| format!("no profile named '{name}'"))?;
    let include = actions::enable_auto(p)?;
    println!("Auto-switch enabled for '{name}'.");
    if let Some(glob) = &p.remote_match {
        println!("  when a repo has a remote matching: {glob}");
    }
    println!("  git will include: {}", include.display());
    Ok(())
}

fn cmd_auto_disable(name: &str) -> Result<()> {
    let reg = Registry::load()?;
    let p = reg
        .get(name)
        .with_context(|| format!("no profile named '{name}'"))?;
    actions::disable_auto(p)?;
    println!("Auto-switch disabled for '{name}'.");
    Ok(())
}

fn cmd_auto_status() -> Result<()> {
    let reg = Registry::load()?;
    let mut any = false;
    for p in &reg.profiles {
        let Some(glob) = &p.remote_match else {
            continue;
        };
        let key = git::auto_key(glob);
        if git::get(Scope::Global, &key)?.is_some() {
            println!("{:<12} enabled  ⇄ {glob}", p.name);
            any = true;
        }
    }
    if !any {
        println!("No profiles have auto-switch enabled.");
    }
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
