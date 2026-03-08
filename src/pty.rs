use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

/// Events produced by the PTY reader thread.
pub enum PtyEvent {
    /// Raw output bytes from the child process.
    Output(Vec<u8>),
    /// The child process has exited.
    Exited,
}

/// Manages a PTY-spawned child process with non-blocking output reading.
pub struct PtyHandle {
    #[allow(dead_code)]
    child: Box<dyn portable_pty::Child + Send>,
    writer: Box<dyn Write + Send>,
    reader_rx: mpsc::Receiver<PtyEvent>,
    shutdown: Arc<AtomicBool>,
    master_pty: Box<dyn portable_pty::MasterPty + Send>,
    #[allow(dead_code)]
    reader_thread: Option<std::thread::JoinHandle<()>>,
}

impl PtyHandle {
    /// Spawn a command in a new PTY.
    pub fn spawn(command: &str, args: &[&str], cwd: &Path, cols: u16, rows: u16) -> Result<Self> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let mut cmd = CommandBuilder::new(command);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.cwd(cwd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn command in PTY")?;

        let writer = pair
            .master
            .take_writer()
            .context("Failed to get PTY writer")?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("Failed to get PTY reader")?;

        let (tx, rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();

        let reader_thread = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                if shutdown_clone.load(Ordering::Relaxed) {
                    break;
                }
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF: child process closed its end
                        let _ = tx.send(PtyEvent::Exited);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(PtyEvent::Exited);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            child,
            writer,
            reader_rx: rx,
            shutdown,
            master_pty: pair.master,
            reader_thread: Some(reader_thread),
        })
    }

    /// Write bytes to the PTY (sends input to the child process).
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer
            .write_all(data)
            .context("Failed to write to PTY")?;
        self.writer.flush().context("Failed to flush PTY writer")?;
        Ok(())
    }

    /// Non-blocking drain of all pending PTY events.
    pub fn try_recv(&self) -> Vec<PtyEvent> {
        self.reader_rx.try_iter().collect()
    }

    /// Resize the PTY.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master_pty
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY")?;
        Ok(())
    }
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Reader thread will exit on next read attempt or when PTY closes
    }
}
