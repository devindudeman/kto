//! kto - A generic, flexible web change watcher CLI tool

use clap::Parser;

use kto::cli::{Cli, Commands, NotifyCommands, ProfileCommands, RemindCommands, ServiceCommands};
use kto::error::Result;

mod commands;
mod utils;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // Watch management
        Commands::New {
            description,
            name,
            interval,
            js,
            rss,
            shell,
            agent,
            agent_instructions,
            selector,
            clipboard,
            tag,
            use_profile,
            research,
            yes,
        } => commands::cmd_new(
            description, name, interval, js, rss, shell, agent,
            agent_instructions, selector, clipboard, tag, use_profile, research, yes,
        ),

        Commands::List { verbose, tag, json } => commands::cmd_list(verbose, tag, json),
        Commands::Show { watch, json } => commands::cmd_show(&watch, json),

        Commands::Edit {
            watch,
            name,
            interval,
            enabled,
            agent,
            agent_instructions,
            selector,
            engine,
            extraction,
            notify,
            use_profile,
        } => commands::cmd_edit(
            &watch, name, interval, enabled, agent,
            agent_instructions, selector, engine, extraction, notify, use_profile,
        ),

        Commands::Pause { watch } => commands::cmd_pause(&watch),
        Commands::Resume { watch } => commands::cmd_resume(&watch),
        Commands::Delete { watch, yes } => commands::cmd_delete(&watch, yes),

        // Check and monitoring
        Commands::Test { watch, json } => commands::cmd_test(&watch, json),
        Commands::Watch { url, interval, selector, js } => {
            commands::cmd_watch(&url, &interval, selector, js)
        }
        Commands::Preview { url, selector, js, full, json_ld, limit } => {
            commands::cmd_preview(&url, selector, js, full, json_ld, limit)
        }
        Commands::History { watch, limit, json } => commands::cmd_history(&watch, limit, json),
        Commands::Run => commands::cmd_run(),
        Commands::Daemon => commands::cmd_daemon(),

        // Notification commands
        Commands::Notify(NotifyCommands::Set {
            ntfy,
            slack,
            discord,
            gotify_server,
            gotify_token,
            command,
            telegram_token,
            telegram_chat,
            pushover_user,
            pushover_token,
            matrix_server,
            matrix_room,
            matrix_token,
        }) => commands::cmd_notify_set(
            ntfy, slack, discord, gotify_server, gotify_token, command,
            telegram_token, telegram_chat, pushover_user, pushover_token,
            matrix_server, matrix_room, matrix_token,
        ),
        Commands::Notify(NotifyCommands::Show) => commands::cmd_notify_show(),
        Commands::Notify(NotifyCommands::Test) => commands::cmd_notify_test(),
        Commands::Notify(NotifyCommands::Quiet { start, end, disable }) => {
            commands::cmd_notify_quiet(start, end, disable)
        }

        // Reminder commands
        Commands::Remind(RemindCommands::New {
            message,
            r#in,
            at,
            every,
            note,
        }) => commands::cmd_remind_new(message, r#in, at, every, note),
        Commands::Remind(RemindCommands::List { json }) => commands::cmd_remind_list(json),
        Commands::Remind(RemindCommands::Delete { reminder }) => {
            commands::cmd_remind_delete(reminder)
        }
        Commands::Remind(RemindCommands::Pause { reminder }) => {
            commands::cmd_remind_pause(reminder)
        }
        Commands::Remind(RemindCommands::Resume { reminder }) => {
            commands::cmd_remind_resume(reminder)
        }

        // Profile commands
        Commands::Profile(ProfileCommands::Show { json }) => commands::cmd_profile_show(json),
        Commands::Profile(ProfileCommands::Edit) => commands::cmd_profile_edit(),
        Commands::Profile(ProfileCommands::Setup) => commands::cmd_profile_setup(),
        Commands::Profile(ProfileCommands::Infer { yes }) => commands::cmd_profile_infer(yes),
        Commands::Profile(ProfileCommands::Preview { watch }) => {
            commands::cmd_profile_preview(&watch)
        }
        Commands::Profile(ProfileCommands::Clear { yes }) => commands::cmd_profile_clear(yes),
        Commands::Profile(ProfileCommands::Forget { learned, yes }) => {
            commands::cmd_profile_forget(learned, yes)
        }

        // Service management
        Commands::Service(ServiceCommands::Install { cron, cron_interval }) => {
            commands::cmd_service_install(cron, cron_interval)
        }
        Commands::Service(ServiceCommands::Uninstall) => commands::cmd_service_uninstall(),
        Commands::Service(ServiceCommands::Status) => commands::cmd_service_status(),
        Commands::Service(ServiceCommands::Logs { lines, follow }) => {
            commands::cmd_service_logs(lines, follow)
        }

        // Miscellaneous
        Commands::Doctor => commands::cmd_doctor(),
        Commands::EnableJs => commands::cmd_enable_js(),
        Commands::Ui => commands::cmd_ui(),
        Commands::Export { watch } => commands::cmd_export(watch),
        Commands::Import { dry_run } => commands::cmd_import(dry_run),
        Commands::Diff { watch, limit } => commands::cmd_diff(&watch, limit),
        Commands::Memory { watch, json, clear } => commands::cmd_memory(&watch, json, clear),
        Commands::Logs { lines, follow, json } => commands::cmd_logs(lines, follow, json),
        Commands::Completions { shell } => commands::cmd_completions(shell),
        Commands::Init => commands::cmd_init(),
    }
}
