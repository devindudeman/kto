//! Service management commands: install, uninstall, status, logs

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use kto::error::Result;

/// Detect which service manager to use
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum ServiceManager {
    Systemd,
    Launchd,
    Cron,
}

#[allow(dead_code)]
impl ServiceManager {
    pub fn detect() -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            // Check if systemd is available
            if Command::new("systemctl")
                .arg("--user")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(ServiceManager::Systemd);
            }
        }

        #[cfg(target_os = "macos")]
        {
            // launchd is always available on macOS
            return Some(ServiceManager::Launchd);
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            None
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        None
    }

    pub fn name(&self) -> &'static str {
        match self {
            ServiceManager::Systemd => "systemd",
            ServiceManager::Launchd => "launchd",
            ServiceManager::Cron => "cron",
        }
    }
}

fn get_kto_binary_path() -> Result<String> {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| kto::KtoError::ConfigError(format!("Could not determine kto path: {}", e)))
}

fn systemd_service_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join("kto.service")
}

fn launchd_plist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join("com.kto.daemon.plist")
}

fn generate_systemd_service(kto_path: &str) -> String {
    format!(r#"[Unit]
Description=kto web change watcher daemon
After=network.target

[Service]
Type=simple
ExecStart={kto_path} daemon
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
"#)
}

fn generate_launchd_plist(kto_path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let log_path = format!("{}/Library/Logs/kto.log", home);
    let err_path = format!("{}/Library/Logs/kto.error.log", home);

    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.kto.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{kto_path}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log_path}</string>
    <key>StandardErrorPath</key>
    <string>{err_path}</string>
</dict>
</plist>
"#)
}

fn install_systemd_service() -> Result<()> {
    let kto_path = get_kto_binary_path()?;
    let service_path = systemd_service_path();

    // Create directory if needed
    if let Some(parent) = service_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write service file
    let service_content = generate_systemd_service(&kto_path);
    std::fs::write(&service_path, service_content)?;

    println!("  Created {}", service_path.display());

    // Reload systemd
    let reload = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()?;

    if !reload.success() {
        return Err(kto::KtoError::ConfigError("Failed to reload systemd".into()));
    }

    // Enable and start service
    let enable = Command::new("systemctl")
        .args(["--user", "enable", "kto.service"])
        .status()?;

    if !enable.success() {
        return Err(kto::KtoError::ConfigError("Failed to enable service".into()));
    }

    let start = Command::new("systemctl")
        .args(["--user", "start", "kto.service"])
        .status()?;

    if !start.success() {
        return Err(kto::KtoError::ConfigError("Failed to start service".into()));
    }

    println!("  Service enabled and started");
    println!("\n  Commands:");
    println!("    systemctl --user status kto");
    println!("    systemctl --user stop kto");
    println!("    systemctl --user restart kto");
    println!("    journalctl --user -u kto -f");

    Ok(())
}

fn install_launchd_service() -> Result<()> {
    let kto_path = get_kto_binary_path()?;
    let plist_path = launchd_plist_path();

    // Create directory if needed
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write plist file
    let plist_content = generate_launchd_plist(&kto_path);
    std::fs::write(&plist_path, plist_content)?;

    println!("  Created {}", plist_path.display());

    // Load the service
    let load = Command::new("launchctl")
        .args(["load", "-w", &plist_path.to_string_lossy()])
        .status()?;

    if !load.success() {
        return Err(kto::KtoError::ConfigError("Failed to load service".into()));
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    println!("  Service loaded and started");
    println!("\n  Commands:");
    println!("    launchctl list | grep kto");
    println!("    launchctl unload ~/Library/LaunchAgents/com.kto.daemon.plist");
    println!("    tail -f ~/Library/Logs/kto.log");
    println!("\n  Logs: {}/Library/Logs/kto.log", home);

    Ok(())
}

fn install_cron_service(interval_mins: u32) -> Result<()> {
    let kto_path = get_kto_binary_path()?;

    // Get current crontab
    let current = Command::new("crontab")
        .arg("-l")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Check if kto is already in crontab and filter out existing entries
    let current = if current.contains("kto run") {
        println!("  kto is already in crontab. Updating...");
        current
            .lines()
            .filter(|line| !line.contains("kto run"))
            .collect::<Vec<&str>>()
            .join("\n")
    } else {
        current
    };

    // Add new cron entry
    let cron_entry = format!("*/{} * * * * {} run >> /tmp/kto-cron.log 2>&1", interval_mins, kto_path);
    let new_crontab = if current.is_empty() {
        cron_entry
    } else {
        format!("{}\n{}", current.trim(), cron_entry)
    };

    // Install new crontab
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(new_crontab.as_bytes())?;
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(kto::KtoError::ConfigError("Failed to install crontab".into()));
    }

    println!("  Added to crontab: run every {} minutes", interval_mins);
    println!("\n  Commands:");
    println!("    crontab -l              # View crontab");
    println!("    crontab -e              # Edit crontab");
    println!("    tail -f /tmp/kto-cron.log  # View logs");

    Ok(())
}

/// Install kto as a background service
pub fn cmd_service_install(use_cron: bool, cron_interval: u32) -> Result<()> {
    println!("\nInstalling kto service...\n");

    if use_cron {
        install_cron_service(cron_interval)?;
    } else {
        match ServiceManager::detect() {
            Some(ServiceManager::Systemd) => {
                println!("  Detected: systemd (Linux)");
                install_systemd_service()?;
            }
            Some(ServiceManager::Launchd) => {
                println!("  Detected: launchd (macOS)");
                install_launchd_service()?;
            }
            _ => {
                println!("  No native service manager detected.");
                println!("  Installing via cron instead...\n");
                install_cron_service(cron_interval)?;
            }
        }
    }

    println!("\n  kto is now running in the background!");
    println!("  Use `kto service status` to check status.");

    Ok(())
}

/// Uninstall the background service
pub fn cmd_service_uninstall() -> Result<()> {
    println!("\nUninstalling kto service...\n");

    let mut uninstalled = false;

    // Try systemd
    let systemd_path = systemd_service_path();
    if systemd_path.exists() {
        println!("  Stopping systemd service...");
        let _ = Command::new("systemctl")
            .args(["--user", "stop", "kto.service"])
            .status();
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "kto.service"])
            .status();
        std::fs::remove_file(&systemd_path)?;
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        println!("  Removed systemd service");
        uninstalled = true;
    }

    // Try launchd
    let launchd_path = launchd_plist_path();
    if launchd_path.exists() {
        println!("  Unloading launchd service...");
        let _ = Command::new("launchctl")
            .args(["unload", "-w", &launchd_path.to_string_lossy()])
            .status();
        std::fs::remove_file(&launchd_path)?;
        println!("  Removed launchd service");
        uninstalled = true;
    }

    // Try cron
    if let Ok(output) = Command::new("crontab").arg("-l").output() {
        if output.status.success() {
            let current = String::from_utf8_lossy(&output.stdout);
            if current.contains("kto run") {
                println!("  Removing from crontab...");
                let filtered: Vec<&str> = current
                    .lines()
                    .filter(|line| !line.contains("kto run"))
                    .collect();
                let new_crontab = filtered.join("\n");

                let mut child = Command::new("crontab")
                    .arg("-")
                    .stdin(Stdio::piped())
                    .spawn()?;

                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(new_crontab.as_bytes())?;
                }

                let _ = child.wait();
                println!("  Removed from crontab");
                uninstalled = true;
            }
        }
    }

    if uninstalled {
        println!("\n  kto service uninstalled.");
    } else {
        println!("  No kto service installation found.");
    }

    Ok(())
}

/// Show service status
pub fn cmd_service_status() -> Result<()> {
    println!("\nkto service status:\n");

    let mut found = false;

    // Check systemd
    #[cfg(target_os = "linux")]
    {
        if systemd_service_path().exists() {
            found = true;
            println!("  Type: systemd");
            let output = Command::new("systemctl")
                .args(["--user", "status", "kto.service", "--no-pager"])
                .output()?;
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
    }

    // Check launchd
    #[cfg(target_os = "macos")]
    {
        if launchd_plist_path().exists() {
            found = true;
            println!("  Type: launchd");
            let output = Command::new("launchctl")
                .args(["list"])
                .output()?;
            let list = String::from_utf8_lossy(&output.stdout);
            for line in list.lines() {
                if line.contains("com.kto.daemon") {
                    println!("  {}", line);
                }
            }

            // Check if actually running
            if list.contains("com.kto.daemon") {
                println!("  Status: Running");
            } else {
                println!("  Status: Not running (plist exists but not loaded)");
            }
        }
    }

    // Check cron
    if let Ok(output) = Command::new("crontab").arg("-l").output() {
        if output.status.success() {
            let current = String::from_utf8_lossy(&output.stdout);
            for line in current.lines() {
                if line.contains("kto run") {
                    found = true;
                    println!("  Type: cron");
                    println!("  Entry: {}", line);
                }
            }
        }
    }

    if !found {
        println!("  No kto service installed.");
        println!("  Run `kto service install` to set up background monitoring.");
    }

    Ok(())
}

/// Show service logs
pub fn cmd_service_logs(lines: usize, follow: bool) -> Result<()> {
    // Determine which service is installed and show appropriate logs

    // Check systemd first
    #[cfg(target_os = "linux")]
    {
        if systemd_service_path().exists() {
            let mut cmd = Command::new("journalctl");
            cmd.args(["--user", "-u", "kto.service", "-n", &lines.to_string()]);
            if follow {
                cmd.arg("-f");
            }
            cmd.status()?;
            return Ok(());
        }
    }

    // Check launchd
    #[cfg(target_os = "macos")]
    {
        if launchd_plist_path().exists() {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let log_path = format!("{}/Library/Logs/kto.log", home);

            if std::path::Path::new(&log_path).exists() {
                let mut cmd = if follow {
                    let mut c = Command::new("tail");
                    c.args(["-f", "-n", &lines.to_string(), &log_path]);
                    c
                } else {
                    let mut c = Command::new("tail");
                    c.args(["-n", &lines.to_string(), &log_path]);
                    c
                };
                cmd.status()?;
                return Ok(());
            } else {
                println!("Log file not found: {}", log_path);
                println!("The service may not have started yet.");
                return Ok(());
            }
        }
    }

    // Check cron logs
    let cron_log = "/tmp/kto-cron.log";
    if std::path::Path::new(cron_log).exists() {
        let mut cmd = if follow {
            let mut c = Command::new("tail");
            c.args(["-f", "-n", &lines.to_string(), cron_log]);
            c
        } else {
            let mut c = Command::new("tail");
            c.args(["-n", &lines.to_string(), cron_log]);
            c
        };
        cmd.status()?;
        return Ok(());
    }

    println!("No service logs found.");
    println!("Run `kto service install` to set up background monitoring.");

    Ok(())
}
