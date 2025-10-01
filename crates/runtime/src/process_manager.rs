use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::timeout;
use tracing::{debug, warn, error, info};
use chatsafe_common::Result;

/// Improved process management with proper cleanup
pub struct ProcessManager {
    child: Option<Child>,
    name: String,
}

impl ProcessManager {
    pub fn new(name: String) -> Self {
        Self {
            child: None,
            name,
        }
    }

    /// Spawn a new process with proper stdout/stderr handling
    pub async fn spawn(&mut self, mut command: Command) -> Result<()> {
        // Ensure old process is cleaned up first
        self.cleanup().await?;

        // Configure process with piped outputs for draining
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);  // Ensure process is killed if handle is dropped

        let mut child = command.spawn()?;
        
        // Spawn tasks to drain stdout and stderr to prevent blocking
        if let Some(stdout) = child.stdout.take() {
            let name = self.name.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!("{} stdout: {}", name, line);
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let name = self.name.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!("{} stderr: {}", name, line);
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
                    warn!("Error checking process status: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Gracefully terminate the process with proper cleanup
    pub async fn terminate(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("Terminating {} process", self.name);
            
            // First try SIGTERM for graceful shutdown
            #[cfg(unix)]
            {
                use nix::sys::signal::{self, Signal};
                use nix::unistd::Pid;
                
                if let Some(pid) = child.id() {
                    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
                    
                    // Give it 5 seconds to exit gracefully
                    match timeout(Duration::from_secs(5), child.wait()).await {
                        Ok(Ok(status)) => {
                            info!("{} exited gracefully with status: {:?}", self.name, status);
                            return Ok(());
                        }
                        _ => {
                            // Continue to forceful kill
                            warn!("{} didn't exit gracefully, forcing kill", self.name);
                        }
                    }
                }
            }
            
            // Forceful kill if graceful didn't work
            if let Err(e) = child.kill().await {
                warn!("Failed to kill {}: {}", self.name, e);
            }
            
            // Wait for process to actually exit
            match timeout(Duration::from_secs(2), child.wait()).await {
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
        
        Ok(())
    }

    /// Clean up any existing process
    pub async fn cleanup(&mut self) -> Result<()> {
        if self.is_running() {
            self.terminate().await?;
        }
        self.child = None;
        Ok(())
    }

    /// Get process ID if running
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref()?.id()
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        // Try to kill process on drop
        if let Some(mut child) = self.child.take() {
            let name = self.name.clone();
            tokio::spawn(async move {
                if let Err(e) = child.kill().await {
                    warn!("Failed to kill {} on drop: {}", name, e);
                }
                let _ = child.wait().await;
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
        
        assert!(pm.spawn(cmd).await.is_ok());
        assert!(pm.is_running());
        
        // Terminate it
        assert!(pm.terminate().await.is_ok());
        assert!(!pm.is_running());
    }

    #[tokio::test]
    async fn test_cleanup_on_drop() {
        {
            let mut pm = ProcessManager::new("test".to_string());
            let mut cmd = Command::new("sleep");
            cmd.arg("10");
            let _ = pm.spawn(cmd).await;
            // pm will be dropped here
        }
        
        // Give it time to cleanup
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Process should be gone (we can't easily verify this in test)
    }
}