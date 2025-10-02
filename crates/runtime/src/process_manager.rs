//! Process management with proper cleanup and output draining
//!
//! This module provides a robust process manager that ensures child processes
//! are properly cleaned up and their output streams are drained to prevent deadlock.

use chatsafe_common::Result;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;
use tracing::{error, info, warn};

// Constants
const GRACEFUL_SHUTDOWN_TIMEOUT_SECS: u64 = 5;
const FORCEFUL_KILL_TIMEOUT_SECS: u64 = 2;
const LOG_PREFIX_STDOUT: &str = "stdout";
const LOG_PREFIX_STDERR: &str = "stderr";

/// Process manager that handles spawning, monitoring, and cleanup of child processes
///
/// This ensures:
/// - Stdout/stderr are properly drained to prevent deadlock
/// - Graceful shutdown is attempted before forceful kill
/// - Processes are cleaned up on drop
pub struct ProcessManager {
    child: Option<Child>,
    name: String,
}

impl ProcessManager {
    /// Create a new process manager with the given name for logging
    pub fn new(name: String) -> Self {
        Self { child: None, name }
    }

    /// Spawn a new process with proper stdout/stderr handling
    ///
    /// This will:
    /// 1. Clean up any existing process
    /// 2. Configure the new process with piped outputs
    /// 3. Spawn background tasks to drain stdout/stderr
    pub async fn spawn(&mut self, mut command: Command) -> Result<()> {
        // Ensure old process is cleaned up first
        self.cleanup().await?;

        // Configure process with piped outputs for draining
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true); // Ensure process is killed if handle is dropped

        let mut child = command.spawn()?;

        // Spawn tasks to drain stdout and stderr to prevent blocking
        if let Some(stdout) = child.stdout.take() {
            let name = self.name.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    info!("{} {}: {}", name, LOG_PREFIX_STDOUT, line);
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let name = self.name.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Use warn for stderr as it often contains important diagnostics
                    warn!("{} {}: {}", name, LOG_PREFIX_STDERR, line);
                }
            });
        }

        self.child = Some(child);
        Ok(())
    }

    /// Check if process is still running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process has exited
                    self.child = None;
                    false
                }
                Ok(None) => {
                    // Still running
                    true
                }
                Err(e) => {
                    warn!("Error checking {} process status: {}", self.name, e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Gracefully terminate the process with proper cleanup
    ///
    /// Attempts graceful shutdown with SIGTERM first (on Unix),
    /// then falls back to forceful kill if needed.
    pub async fn terminate(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("Terminating {} process", self.name);

            // Try graceful shutdown first
            let graceful_shutdown = self.try_graceful_shutdown(&mut child).await;

            if graceful_shutdown {
                return Ok(());
            }

            // Forceful kill if graceful didn't work
            self.force_kill(child).await;
        }

        Ok(())
    }

    /// Attempt graceful shutdown (Unix only)
    #[cfg(unix)]
    async fn try_graceful_shutdown(&self, child: &mut Child) -> bool {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        if let Some(pid) = child.id() {
            let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);

            // Give it time to exit gracefully
            match timeout(
                Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS),
                child.wait(),
            )
            .await
            {
                Ok(Ok(status)) => {
                    info!("{} exited gracefully with status: {:?}", self.name, status);
                    return true;
                }
                _ => {
                    warn!("{} didn't exit gracefully, will force kill", self.name);
                }
            }
        }
        false
    }

    /// Attempt graceful shutdown (Windows - just returns false to trigger force kill)
    #[cfg(not(unix))]
    async fn try_graceful_shutdown(&self, _child: &mut Child) -> bool {
        // Windows doesn't have SIGTERM, go straight to force kill
        false
    }

    /// Force kill a process
    async fn force_kill(&self, mut child: Child) {
        if let Err(e) = child.kill().await {
            warn!("Failed to kill {}: {}", self.name, e);
        }

        // Wait for process to actually exit
        match timeout(
            Duration::from_secs(FORCEFUL_KILL_TIMEOUT_SECS),
            child.wait(),
        )
        .await
        {
            Ok(Ok(status)) => {
                info!("{} forcefully killed with status: {:?}", self.name, status);
            }
            Ok(Err(e)) => {
                error!("Error waiting for {} to exit: {}", self.name, e);
            }
            Err(_) => {
                error!("Timeout waiting for {} to exit after kill", self.name);
            }
        }
    }

    /// Clean up any existing process
    pub async fn cleanup(&mut self) -> Result<()> {
        if self.is_running() {
            self.terminate().await?;
        }
        self.child = None;
        Ok(())
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        // Try to kill process on drop
        // Note: We can't await in drop, so we spawn a detached task
        if let Some(mut child) = self.child.take() {
            let name = self.name.clone();

            // Use spawn_blocking for the kill operation in drop context
            std::thread::spawn(move || {
                let rt = tokio::runtime::Handle::try_current();
                if let Ok(handle) = rt {
                    handle.spawn(async move {
                        if let Err(e) = child.kill().await {
                            warn!("Failed to kill {} on drop: {}", name, e);
                        }
                        let _ = child.wait().await;
                    });
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_lifecycle() {
        let mut pm = ProcessManager::new("test".to_string());

        // Spawn a simple process
        let mut cmd = Command::new("sleep");
        cmd.arg("10");

        pm.spawn(cmd).await.expect("Failed to spawn process");
        assert!(pm.is_running());

        // Terminate it
        pm.terminate().await.expect("Failed to terminate process");
        assert!(!pm.is_running());
    }

    #[tokio::test]
    async fn test_cleanup_on_drop() {
        {
            let mut pm = ProcessManager::new("test_drop".to_string());
            let mut cmd = Command::new("sleep");
            cmd.arg("10");
            pm.spawn(cmd).await.expect("Failed to spawn process");
            // pm will be dropped here
        }

        // Give it time to cleanup
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Process should be gone (we can't easily verify this in test)
        // but at least we verify no panic occurred
    }

    #[tokio::test]
    async fn test_multiple_spawn() {
        let mut pm = ProcessManager::new("test_multi".to_string());

        // Spawn first process
        let mut cmd1 = Command::new("sleep");
        cmd1.arg("10");
        pm.spawn(cmd1).await.expect("Failed to spawn first process");
        assert!(pm.is_running());

        // Spawn second process (should cleanup first)
        let mut cmd2 = Command::new("sleep");
        cmd2.arg("10");
        pm.spawn(cmd2)
            .await
            .expect("Failed to spawn second process");
        assert!(pm.is_running());

        // Cleanup
        pm.cleanup().await.expect("Failed to cleanup");
        assert!(!pm.is_running());
    }
}
